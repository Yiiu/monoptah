use super::*;
use gpui::{
    Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions, point,
    size,
};

fn window(section: Section, cx: &mut TestAppContext) -> WindowHandle<SettingsView> {
    let window = cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1200.), px(800.)),
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| {
                    let mut view = SettingsView::new(
                        section,
                        Preferences::default(),
                        SettingsContext::default(),
                        cx,
                    );
                    view.discovery_task = None;
                    view.discovery_loading = false;
                    view
                })
            },
        )
        .unwrap()
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    window
}

#[gpui::test]
fn workspace_style_is_only_offered_under_the_new_layout(cx: &mut TestAppContext) {
    let window = window(Section::Appearance, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for classic in [false, true, false] {
        window
            .update(visual, |view, _, cx| {
                view.ctx.classic = classic;
                view.ctx.new_layout_boxy = true;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual
                .debug_bounds("settings-new-layout-style-boxy")
                .is_some(),
            !classic
        );
        assert_eq!(
            visual
                .debug_bounds("settings-new-layout-style-rounded")
                .is_some(),
            !classic
        );
        assert!(
            visual
                .debug_bounds("settings-collapsed-project-rail-hidden")
                .is_none()
        );
    }
}

#[gpui::test]
fn skills_document_has_its_own_full_height_pane_and_compact_actions(cx: &mut TestAppContext) {
    let window = window(Section::Skills, cx);
    window
        .update(cx, |view, _, cx| {
            let path = std::path::PathBuf::from("/project/.agents/skills/review/SKILL.md");
            view.discovery.skills = vec![data::Skill {
                name: "review".into(),
                description: "Review changes".into(),
                path: path.clone(),
                scope: "project".into(),
                provider: "agents".into(),
            }];
            view.skill_preview = Some(path);
            view.skill_preview_text = Some(Ok(skill_document::DocumentData::new(format!(
                "---\nname: review\n---\n# Review\n\n{}",
                "Instructions.\n\n".repeat(150)
            ))));
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let left = visual.debug_bounds("skills-list-scroll").unwrap();
    let right = visual.debug_bounds("skill-preview-pane").unwrap();
    let body = visual.debug_bounds("skill-document").unwrap();
    assert_eq!(left.size.height, px(800.));
    assert_eq!(right.size.height, px(800.));
    assert!(right.left() >= left.right());
    assert!(body.bottom() <= right.bottom());
    assert!(body.size.height > px(600.));
    assert!(visual.debug_bounds("transcript").is_none());
    for id in [
        "skill-preview-action",
        "skill-copy-action",
        "skill-reveal-action",
    ] {
        assert_eq!(
            visual.debug_bounds(id).unwrap().size,
            size(px(20.), px(20.))
        );
    }
    let source = visual
        .debug_bounds("settings-skill-document-source")
        .unwrap();
    assert!(source.size.width < px(100.));
    visual.simulate_click(source.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.prefs.skill_source_paths.len(), 1)
        })
        .unwrap();
    visual.simulate_resize(size(px(700.), px(800.)));
    visual.run_until_parked();
    // Deliver the frame that applies the measured Skills container breakpoint.
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    visual.run_until_parked();
    let left = visual.debug_bounds("skills-list-scroll").unwrap();
    let right = visual.debug_bounds("skill-preview-pane").unwrap();
    assert!(
        right.top() >= left.bottom(),
        "list={left:?}, preview={right:?}"
    );
    assert_eq!(left.size.width, px(700.));
    assert_eq!(right.size.width, px(700.));
    assert!(visual.debug_bounds("skill-document").unwrap().size.height > px(200.));
}

#[gpui::test]
fn mono_mascot_is_beside_name_and_sidebar_switch_stays_visible(cx: &mut TestAppContext) {
    let window = window(Section::Monos, cx);
    window
        .update(cx, |view, _, cx| {
            view.ctx.monos = vec![MonoSetting {
                id: "layout-mono".into(),
                name: "Big Boss".into(),
                mascot: "ghost".into(),
                color: 0xee99bb,
                projects: vec!["monocode".into(), "mono-web".into()],
                show: false,
                folders: false,
            }];
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let mascot = visual.debug_bounds("settings-mono-mascot").unwrap();
    let switch = visual
        .debug_bounds("settings-toggle-mono-sidebar-show")
        .unwrap();
    let reset = visual.debug_bounds("settings-mono-reset").unwrap();
    assert_eq!(mascot.size, size(px(16.), px(16.)));
    assert_eq!(switch.size, size(px(36.), px(20.)));
    let name = visual.debug_bounds("settings-mono-name").unwrap();
    assert!(mascot.right() < name.left());
    assert!(mascot.left() < px(200.));
    assert!(switch.left() > px(800.));
    assert!(switch.right() <= reset.left());
    assert!(reset.right() <= px(1200.));
    assert!((mascot.top() - switch.top()).abs() < px(4.));
    assert!(
        visual
            .debug_bounds("settings-toggle-mono-sidebar-folders")
            .is_none()
    );
    visual.simulate_click(reset.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("settings-management-dialog").is_some());
    window
        .update(visual, |view, window, cx| {
            view.close_management(window, cx);
            cx.notify();
        })
        .unwrap();
    visual.simulate_resize(size(px(700.), px(800.)));
    visual.run_until_parked();
    let switch = visual
        .debug_bounds("settings-toggle-mono-sidebar-show")
        .unwrap();
    assert_eq!(switch.size, size(px(36.), px(20.)));
    assert!(switch.right() <= px(700.));
}

fn tree(path: &str, branch: Option<&str>, locked: bool) -> data::Tree {
    data::Tree {
        tree: monocode_core::git::Worktree {
            path: path.into(),
            branch: branch.map(Into::into),
            ..Default::default()
        },
        locked,
        dirty: Some(false),
        unpushed: 0,
    }
}

fn worktree_window(cx: &mut TestAppContext) -> WindowHandle<SettingsView> {
    let window = window(Section::Worktrees, cx);
    window
        .update(cx, |view, _, cx| {
            view.selected_project = "/repo".into();
            view.discovery.worktrees.main = "/repo".into();
            view.discovery.worktrees.trees = vec![
                tree("/wt/a", Some("a"), false),
                tree("/wt/locked", Some("locked"), true),
                tree("/wt/detached", None, false),
                tree("/wt/b", Some("b"), false),
            ];
            cx.notify();
        })
        .unwrap();
    window
}

#[gpui::test]
fn worktrees_select_all_skips_locked_and_detached_trees(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let click_select_all = |visual: &mut VisualTestContext| {
        let bounds = visual.debug_bounds("select-all-trees").unwrap();
        visual.simulate_click(bounds.center(), Default::default());
    };
    click_select_all(visual);
    window
        .update(visual, |view, _, _| {
            let mut got = view.selected_trees.iter().cloned().collect::<Vec<_>>();
            got.sort();
            assert_eq!(got, ["/wt/a", "/wt/b"]);
        })
        .unwrap();
    // Blocked rows ignore clicks.
    let locked = visual.debug_bounds("select-tree-1").unwrap();
    visual.simulate_click(locked.center(), Default::default());
    // A selected row can be cleared on its own.
    let a = visual.debug_bounds("select-tree-0").unwrap();
    visual.simulate_click(a.center(), Default::default());
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.selected_trees.len(), 1);
            assert!(view.selected_trees.contains("/wt/b"));
        })
        .unwrap();
    click_select_all(visual);
    click_select_all(visual);
    window
        .update(visual, |view, _, _| assert!(view.selected_trees.is_empty()))
        .unwrap();
}

#[gpui::test]
fn selection_follows_the_discovered_worktrees(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, _, cx| {
            view.selected_trees = ["/wt/a", "/wt/b", "/wt/gone"]
                .iter()
                .map(|s| s.to_string())
                .collect();
            view.discovery
                .worktrees
                .trees
                .retain(|t| t.tree.path != "/wt/b");
            view.prune_selected_trees();
            assert_eq!(view.selected_trees.len(), 1);
            assert!(view.selected_trees.contains("/wt/a"));
            view.select_section(Section::Archive, cx);
            assert!(view.selected_trees.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn bulk_delete_runs_one_remove_event_at_a_time_and_keeps_failures_selected(
    cx: &mut TestAppContext,
) {
    let window = worktree_window(cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let view = window.root(visual).unwrap();
    let log = events.clone();
    visual.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &SettingsEvent, _| {
            if let SettingsEvent::RemoveWorktree {
                cwd,
                path,
                delete_sessions,
            } = event
            {
                log.borrow_mut()
                    .push((cwd.clone(), path.clone(), *delete_sessions));
            }
        })
        .detach();
    });
    window
        .update(visual, |view, _, cx| {
            view.selected_trees = ["/wt/a", "/wt/b"].iter().map(|s| s.to_string()).collect();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let button = visual.debug_bounds("delete-selected-trees").unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("settings-management-dialog").is_some());
    window
        .update(visual, |view, _, cx| view.submit_management(cx))
        .unwrap();
    visual.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
    assert_eq!(events.borrow()[0], ("/repo".into(), "/wt/a".into(), false));
    // Busy: a second submit does not start another delete.
    window
        .update(visual, |view, _, cx| view.submit_management(cx))
        .unwrap();
    visual.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
    // The first fails; the run continues with the second.
    window
        .update(visual, |view, _, cx| {
            view.management_finished(Err("in use".into()), Some("/wt/a"), cx)
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(events.borrow().len(), 2);
    assert_eq!(events.borrow()[1].1, "/wt/b");
    window
        .update(visual, |view, _, cx| {
            view.management_finished(Ok(()), Some("/wt/b"), cx)
        })
        .unwrap();
    // Checked before the refresh lands: the test folders don't exist on disk.
    window
        .update(visual, |view, _, _| {
            let editor = view.management.as_ref().expect("summary stays open");
            assert!(!editor.busy);
            assert_eq!(editor.bulk.as_ref().unwrap().failures().len(), 1);
            assert!(view.selected_trees.contains("/wt/a"));
            assert!(!view.selected_trees.contains("/wt/b"));
        })
        .unwrap();
    assert!(visual.debug_bounds("settings-management-dialog").is_some());
    assert_eq!(events.borrow().len(), 2);
}

#[gpui::test]
fn bulk_delete_resets_the_project_and_closes_when_everything_succeeds(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, window, cx| {
            view.selected_project = "/wt/a/sub".into();
            view.selected_trees = ["/wt/a"].iter().map(|s| s.to_string()).collect();
            let chosen = vec![view.discovery.worktrees.trees[0].clone()];
            view.open_management(management::Target::DeleteTrees(chosen), window, cx);
            view.submit_management(cx);
            view.management_finished(Ok(()), Some("/wt/a"), cx);
            assert_eq!(view.selected_project, "/repo");
            assert!(view.management.is_none());
            assert!(view.selected_trees.is_empty());
        })
        .unwrap();
}

fn record_removals(
    window: WindowHandle<SettingsView>,
    visual: &mut VisualTestContext,
) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = events.clone();
    let view = window.root(visual).unwrap();
    visual.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &SettingsEvent, _| {
            if let SettingsEvent::RemoveWorktree { path, .. } = event {
                log.borrow_mut().push(path.clone());
            }
        })
        .detach();
    });
    events
}

#[gpui::test]
fn bulk_delete_ignores_results_for_another_worktree(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let events = record_removals(window, visual);
    window
        .update(visual, |view, window, cx| {
            let chosen = vec![
                view.discovery.worktrees.trees[0].clone(),
                view.discovery.worktrees.trees[3].clone(),
            ];
            view.selected_trees = ["/wt/a", "/wt/b"].iter().map(|s| s.to_string()).collect();
            view.open_management(management::Target::DeleteTrees(chosen), window, cx);
            view.submit_management(cx);
            // A stale result, a result for an item that is still queued, and one
            // with no path at all must not move the run on.
            view.management_finished(Ok(()), Some("/wt/old"), cx);
            view.management_finished(Ok(()), Some("/wt/b"), cx);
            view.management_finished(Ok(()), None, cx);
            let editor = view.management.as_ref().unwrap();
            assert!(editor.busy);
            let bulk = editor.bulk.as_ref().unwrap();
            assert_eq!(bulk.current().unwrap().path, "/wt/a");
            assert!(bulk.deleted().is_empty());
            assert_eq!(view.selected_trees.len(), 2);
        })
        .unwrap();
    assert_eq!(*events.borrow(), ["/wt/a"]);
    window
        .update(visual, |view, _, cx| {
            view.management_finished(Ok(()), Some("/wt/a"), cx)
        })
        .unwrap();
    assert_eq!(*events.borrow(), ["/wt/a", "/wt/b"]);
}

#[gpui::test]
fn single_delete_ignores_results_for_another_worktree(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, window, cx| {
            let tree = view.discovery.worktrees.trees[0].clone();
            view.open_management(management::Target::DeleteTree(tree), window, cx);
            // Not submitted yet: a result can't close the dialog.
            view.management_finished(Ok(()), Some("/wt/a"), cx);
            assert!(view.management.is_some());
            view.submit_management(cx);
            view.management_finished(Ok(()), Some("/wt/b"), cx);
            view.management_finished(Err("late".into()), None, cx);
            let editor = view.management.as_ref().unwrap();
            assert!(editor.busy && editor.error.is_none());
            view.management_finished(Ok(()), Some("/wt/a"), cx);
            assert!(view.management.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn results_without_a_path_still_finish_other_forms(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, window, cx| {
            view.open_management(management::Target::Mcp, window, cx);
            view.management_finished(Err("bad".into()), None, cx);
            assert_eq!(
                view.management.as_ref().unwrap().error.as_deref(),
                Some("bad")
            );
            view.management_finished(Ok(()), None, cx);
            assert!(view.management.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn a_running_delete_blocks_leaving_the_section(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, window, cx| {
            assert!(!view.delete_in_flight());
            let tree = view.discovery.worktrees.trees[0].clone();
            view.open_management(management::Target::DeleteTree(tree), window, cx);
            assert!(!view.delete_in_flight(), "open but not started");
            view.submit_management(cx);
            assert!(view.delete_in_flight());
            view.select_section(Section::Archive, cx);
            assert_eq!(view.section, Section::Worktrees);
            assert!(view.management.is_some());
            view.management_finished(Ok(()), Some("/wt/a"), cx);
            assert!(!view.delete_in_flight());
            view.select_section(Section::Archive, cx);
            assert_eq!(view.section, Section::Archive);
        })
        .unwrap();
}

#[gpui::test]
fn the_dialog_title_reports_a_finished_run_with_failures(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, window, cx| {
            let chosen = vec![
                view.discovery.worktrees.trees[0].clone(),
                view.discovery.worktrees.trees[3].clone(),
            ];
            view.open_management(management::Target::DeleteTrees(chosen), window, cx);
            assert_eq!(view.management_title(), "Delete 2 worktrees?");
            view.submit_management(cx);
            assert_eq!(view.management_title(), "Delete 2 worktrees?");
            view.management_finished(Ok(()), Some("/wt/a"), cx);
            view.management_finished(Err("in use".into()), Some("/wt/b"), cx);
            assert_eq!(view.management_title(), "1 of 2 worktrees deleted");
        })
        .unwrap();
}

#[gpui::test]
fn switching_project_in_the_picker_clears_the_selection(cx: &mut TestAppContext) {
    let window = worktree_window(cx);
    window
        .update(cx, |view, _, cx| {
            view.ctx.projects = vec![
                ("/repo".into(), "repo".into()),
                ("/other".into(), "other".into()),
            ];
            view.selected_trees = ["/wt/a"].iter().map(|s| s.to_string()).collect();
            cx.notify();
        })
        .unwrap();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let trigger = visual
        .debug_bounds("settings-select-settings-project")
        .unwrap();
    visual.simulate_click(trigger.center(), Default::default());
    visual.run_until_parked();
    let option = visual
        .debug_bounds("settings-option-settings-project-1")
        .unwrap();
    visual.simulate_click(option.center(), Default::default());
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.selected_project, "/other");
            assert!(view.selected_trees.is_empty());
        })
        .unwrap();
}

fn cloud_window(cx: &mut TestAppContext) -> WindowHandle<SettingsView> {
    let window = window(Section::General, cx);
    cx.update(|cx| {
        cx.set_global(crate::cloud_kit_ui::KitScanner(
            crate::cloud_kit_ui::fixture::scan,
        ))
    });
    window
        .update(cx, |view, _, cx| {
            view.selected_project = "/work/app".into();
            view.select_section(Section::Cloud, cx);
        })
        .unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    // The Cloud page now starts with the Backends and Docker accounts groups, so the kit
    // sits below an 800 point window; a taller window keeps its rows on screen.
    VisualTestContext::from_window(window.into(), cx).simulate_resize(size(px(1200.), px(2400.)));
    cx.run_until_parked();
    window
}

fn click(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    visual.simulate_click(bounds.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
}

#[gpui::test]
fn the_cloud_page_checks_everything_compatible_and_refuses_local_only_items(
    cx: &mut TestAppContext,
) {
    let window = cloud_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();

    // Defaults: nothing stored, the compatible skill is checked.
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.prefs.cloud_kit.selection, None);
            assert!(matches!(view.cloud.load, pages::CloudLoad::Ready(_)));
        })
        .unwrap();
    assert!(
        visual
            .debug_bounds("cloud-kit-check-skill:user:review")
            .is_some()
    );

    // Unchecking turns the defaults into an explicit list of ids.
    click(visual, "cloud-kit-row-skill:user:review");
    window
        .update(visual, |view, _, _| {
            let selection = view.prefs.cloud_kit.selection.clone().unwrap();
            assert!(!selection.contains("skill:user:review"));
            assert!(selection.contains("skill:user:deploy"));
            assert!(selection.contains("plugin:fmt@market"));
        })
        .unwrap();

    // MCP tab: a local-only server is drawn, with a reason, and clicking it changes nothing.
    click(visual, "settings-cloud-kit-tab-mcp");
    let before = window
        .update(visual, |view, _, _| view.prefs.cloud_kit.clone())
        .unwrap();
    assert!(visual.debug_bounds("cloud-kit-row-mcp:user:mine").is_some());
    click(visual, "cloud-kit-row-mcp:user:mine");
    click(visual, "cloud-kit-check-mcp:user:mine");
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.prefs.cloud_kit, before);
            let selection = view.prefs.cloud_kit.selection.as_ref().unwrap();
            assert!(!selection.contains("mcp:user:mine"));
            assert!(
                selection.contains("mcp:user:github"),
                "secrets are on by default"
            );
            assert!(
                selection.contains("mcp:user:linear"),
                "OAuth-style servers are on by default"
            );
        })
        .unwrap();

    // A selectable server toggles.
    click(visual, "cloud-kit-row-mcp:user:github");
    window
        .update(visual, |view, _, _| {
            let selection = view.prefs.cloud_kit.selection.as_ref().unwrap();
            assert!(!selection.contains("mcp:user:github"));
            // What is stored is ids only.
            let saved = serde_json::to_string(&view.prefs).unwrap();
            assert!(!saved.contains(crate::cloud_kit_ui::fixture::FAKE_SECRET));
        })
        .unwrap();

    // Back to the defaults.
    click(visual, "cloud-kit-reset");
    window
        .update(visual, |view, _, _| {
            assert_eq!(view.prefs.cloud_kit.selection, None);
        })
        .unwrap();
}

#[gpui::test]
fn the_instructions_switch_shows_the_source_and_what_stays_behind(cx: &mut TestAppContext) {
    let window = cloud_window(cx);
    cx.update(|cx| {
        cx.set_global(crate::cloud_kit_ui::KitScanner(
            crate::cloud_kit_ui::fixture::scan_with_instructions,
        ))
    });
    window
        .update(cx, |view, _, cx| view.refresh_cloud(cx))
        .unwrap();
    cx.run_until_parked();
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();

    // On by default, and the import that stays behind is listed.
    window
        .update(visual, |view, _, _| {
            assert!(view.prefs.cloud_kit.instructions)
        })
        .unwrap();
    assert!(
        visual
            .debug_bounds("cloud-kit-instructions-warning-0")
            .is_some()
    );
    click(visual, "settings-toggle-cloud-kit-instructions");
    window
        .update(visual, |view, _, _| {
            assert!(!view.prefs.cloud_kit.instructions)
        })
        .unwrap();
    click(visual, "settings-toggle-cloud-kit-instructions");
    window
        .update(visual, |view, _, _| {
            assert!(view.prefs.cloud_kit.instructions)
        })
        .unwrap();
}

#[gpui::test]
fn the_instructions_switch_is_disabled_without_a_claude_md(cx: &mut TestAppContext) {
    let window = cloud_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    click(visual, "settings-toggle-cloud-kit-instructions");
    window
        .update(visual, |view, _, _| {
            assert!(view.prefs.cloud_kit.instructions)
        })
        .unwrap();
}

#[gpui::test]
fn the_instructions_switch_follows_the_preference_while_the_scan_has_not_answered(
    cx: &mut TestAppContext,
) {
    let window = cloud_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    window
        .update(visual, |view, _, cx| {
            view.cloud.load = pages::CloudLoad::Loading;
            cx.notify();
        })
        .unwrap();
    visual.refresh().unwrap();
    // Not disabled: a click turns the preference off, and nothing claims there is no CLAUDE.md.
    click(visual, "settings-toggle-cloud-kit-instructions");
    window
        .update(visual, |view, _, _| {
            assert!(!view.prefs.cloud_kit.instructions)
        })
        .unwrap();
}

#[gpui::test]
fn the_cloud_notice_shows_once_and_is_stored_as_acknowledged(cx: &mut TestAppContext) {
    let window = cloud_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("cloud-kit-notice").is_none());

    // First time on: the notice opens, and the kit stays off until it is accepted.
    click(visual, "settings-toggle-cloud-kit-enabled");
    assert!(visual.debug_bounds("cloud-kit-notice").is_some());
    window
        .update(visual, |view, _, _| {
            assert!(!view.prefs.cloud_kit.enabled);
            assert!(!view.prefs.cloud_kit.notice_acknowledged);
        })
        .unwrap();

    // Cancel leaves everything as it was.
    click(visual, "cloud-kit-notice-cancel");
    assert!(visual.debug_bounds("cloud-kit-notice").is_none());
    window
        .update(visual, |view, _, _| assert!(!view.prefs.cloud_kit.enabled))
        .unwrap();

    click(visual, "settings-toggle-cloud-kit-enabled");
    click(visual, "cloud-kit-notice-accept");
    assert!(visual.debug_bounds("cloud-kit-notice").is_none());
    window
        .update(visual, |view, _, _| {
            assert!(view.prefs.cloud_kit.enabled);
            assert!(view.prefs.cloud_kit.notice_acknowledged);
        })
        .unwrap();

    // Off and on again: no second notice.
    click(visual, "settings-toggle-cloud-kit-enabled");
    window
        .update(visual, |view, _, _| assert!(!view.prefs.cloud_kit.enabled))
        .unwrap();
    click(visual, "settings-toggle-cloud-kit-enabled");
    assert!(visual.debug_bounds("cloud-kit-notice").is_none());
    window
        .update(visual, |view, _, _| assert!(view.prefs.cloud_kit.enabled))
        .unwrap();
}

#[gpui::test]
fn the_cloud_page_has_a_refresh_button_and_stays_when_preferences_change(cx: &mut TestAppContext) {
    let window = cloud_window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("refresh-cloud").is_some());

    let on = Preferences::default();
    window
        .update(visual, |view, _, cx| view.set_preferences(&on, cx))
        .unwrap();
    window
        .update(visual, |view, _, cx| {
            assert_eq!(view.section, Section::Cloud);
            view.set_preferences(&Preferences::default(), cx);
            assert_eq!(
                view.section,
                Section::Cloud,
                "the page has no flag any more"
            );
        })
        .unwrap();
}

/// The DEFAULT badge marks the account new sessions use, and the pin button on
/// another account makes that one the default and moves the badge to it.
#[gpui::test]
fn the_default_badge_follows_the_account_chosen_with_the_pin_button(cx: &mut TestAppContext) {
    use monocode_core::{accounts::DEFAULT, provider::Kind};
    let _clis = crate::shell::surface_tests::NoClis::new();
    let dir = std::env::temp_dir().join(format!(
        "native-settings-accounts-{}",
        monocode_core::mono::new_id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = std::sync::Arc::new(std::sync::Mutex::new(
        monocode_core::store::Store::open(&dir.join("native.sqlite")).unwrap(),
    ));
    cx.update(|cx| crate::accounts::install(store, cx));
    let account = cx.update(|cx| {
        crate::accounts::get(cx).unwrap().update(cx, |center, _| {
            center.book.add(Kind::Claude, "Token").unwrap().id
        })
    });
    let window = window(Section::Providers, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    let badge = |id: &str| format!("account-default-badge-claude-{id}");
    let pin = |id: &str| format!("account-set-default-claude-{id}");
    let has = |visual: &mut VisualTestContext, selector: String| {
        visual
            .debug_bounds(Box::leak(selector.into_boxed_str()))
            .is_some()
    };
    // The CLI login is the default until another account is chosen.
    assert!(has(visual, badge(DEFAULT)));
    assert!(!has(visual, pin(DEFAULT)));
    assert!(!has(visual, badge(&account)));
    let button = visual
        .debug_bounds(Box::leak(pin(&account).into_boxed_str()))
        .unwrap();
    visual.simulate_click(button.center(), Default::default());
    visual.run_until_parked();
    let preferred = visual.update(|_, cx| {
        crate::accounts::get(cx)
            .unwrap()
            .read(cx)
            .book
            .preferred(Kind::Claude)
    });
    assert_eq!(preferred, account);
    assert!(has(visual, badge(&account)));
    assert!(!has(visual, badge(DEFAULT)));
    // The CLI login can be made the default again.
    assert!(has(visual, pin(DEFAULT)));
    let _ = std::fs::remove_dir_all(dir);
}

// --- Agent CLIs (Settings → Providers).

mod agent_clis {
    use super::{click, window};
    use crate::settings::{
        AgentUpdate, AgentUpdateState, Preferences, Section, SettingsEvent, SettingsView, model,
        preferences::{OPEN_CHAT, OPEN_PER_AGENT, OPEN_TUI},
    };
    use gpui::{TestAppContext, VisualTestContext, WindowHandle, px, size};
    use monocode_core::{agent::Permission, cli::Inspection, provider::Kind, surface::Surface};

    fn fake_cli(kind: Kind) -> std::path::PathBuf {
        std::path::PathBuf::from(format!("/opt/agents/{}", kind.binary_name()))
    }

    fn agent_prefs(open_mode: &str) -> Preferences {
        let mut prefs = Preferences::default();
        prefs.agents.open_mode = open_mode.into();
        prefs.agents.permission = Some(Permission::Own.id().into());
        prefs
    }

    /// The Providers page with `installed` detected, each already checked (so opening a
    /// row never starts a CLI), in a window tall enough to show every row.
    fn agents_window(
        installed: &[Kind],
        prefs: Preferences,
        cx: &mut TestAppContext,
    ) -> WindowHandle<SettingsView> {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            crate::input::bind(cx);
            crate::popover::bind(cx);
        });
        let window = window(Section::Providers, cx);
        let installed = installed.to_vec();
        window
            .update(cx, |view, _, cx| {
                view.prefs = prefs;
                view.discovery.binaries = Kind::ALL
                    .into_iter()
                    .map(|kind| (kind, installed.contains(&kind).then(|| fake_cli(kind))))
                    .collect();
                // The auto-detected CLI "answers" without being started; a command a test
                // types is resolved for real, which starts nothing unless it exists.
                view.agents.inspector = Some(std::sync::Arc::new(|kind, command| match command {
                    Some(_) => monocode_core::cli::inspect(kind, command),
                    None => Ok(Inspection {
                        path: fake_cli(kind),
                        version: "1.2.3".into(),
                    }),
                }));
                for kind in &installed {
                    view.agents.inspections.insert(
                        *kind,
                        Ok(Inspection {
                            path: fake_cli(*kind),
                            version: "1.2.3".into(),
                        }),
                    );
                }
                cx.notify();
            })
            .unwrap();
        VisualTestContext::from_window(window.into(), cx)
            .simulate_resize(size(px(1200.), px(3200.)));
        // Focus changes reach their listeners only in the active window, as the one the
        // user types in is.
        window
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        window
    }

    fn drawn(visual: &mut VisualTestContext, selector: impl Into<String>) -> bool {
        visual
            .debug_bounds(Box::leak(selector.into().into_boxed_str()))
            .is_some()
    }

    fn press(visual: &mut VisualTestContext, selector: impl Into<String>) {
        click(visual, Box::leak(selector.into().into_boxed_str()));
    }

    /// Click a text field and type into it.
    fn type_into(visual: &mut VisualTestContext, selector: impl Into<String>, text: &str) {
        press(visual, selector);
        visual.simulate_input(text);
        visual.run_until_parked();
    }

    /// Let pending work finish and draw its result. A focus listener runs while a frame
    /// is drawn and what it changes shows in the next frame, which the test window draws
    /// at the end of an update: the empty update makes that frame the one read next.
    fn settle(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
        visual.update(|_, _| {});
    }

    fn prefs_of(window: WindowHandle<SettingsView>, visual: &mut VisualTestContext) -> Preferences {
        window
            .update(visual, |view, _, _| view.prefs.clone())
            .unwrap()
    }

    #[gpui::test]
    fn the_open_mode_cards_decide_where_new_sessions_open(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude, Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Search lands on the row above the cards.
        assert!(drawn(visual, "settings-row-agent-open-mode"));
        for mode in [OPEN_CHAT, OPEN_TUI, OPEN_PER_AGENT] {
            assert!(
                drawn(visual, format!("settings-agent-open-mode-{mode}")),
                "{mode}"
            );
        }
        // One mode for every agent: a row has no switch of its own.
        assert!(!drawn(visual, "settings-agent-surface-claude-tui"));

        press(visual, "settings-agent-open-mode-tui");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.open_mode(), OPEN_TUI);
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Tui);
        assert!(!drawn(visual, "settings-agent-surface-claude-tui"));

        // One mode for all rewrites no agent's own surface.
        assert!(prefs.provider_surfaces.is_empty());

        // Choosing per agent shows the switches of the installed agents, each at its own
        // saved surface (chat until one is set).
        press(visual, "settings-agent-open-mode-per-agent");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.open_mode(), OPEN_PER_AGENT);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Chat);
        press(visual, "settings-agent-surface-codex-tui");
        assert_eq!(
            prefs_of(window, visual).agent(Kind::Codex).surface,
            Surface::Tui
        );
        // Per agent → Chat → per agent: every row comes back as it was left.
        press(visual, "settings-agent-open-mode-chat");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Chat);
        assert_eq!(
            prefs.provider_surfaces.get("codex"),
            Some(&Surface::Tui),
            "the saved surface is kept while one mode applies to all"
        );
        press(visual, "settings-agent-open-mode-per-agent");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Tui);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Chat);
        for option in ["chat", "tui"] {
            assert!(drawn(
                visual,
                format!("settings-agent-surface-claude-{option}")
            ));
            assert!(drawn(
                visual,
                format!("settings-agent-surface-codex-{option}")
            ));
            assert!(
                !drawn(visual, format!("settings-agent-surface-grok-{option}")),
                "an agent that is not installed has no switch"
            );
        }
        press(visual, "settings-agent-surface-claude-tui");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Tui);
        assert_eq!(
            prefs.agent(Kind::Codex).surface,
            Surface::Tui,
            "the other agents keep theirs"
        );
    }

    #[gpui::test]
    fn the_open_mode_cards_give_way_to_a_hint_when_terminals_cannot_be_stored(
        cx: &mut TestAppContext,
    ) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_PER_AGENT), cx);
        window
            .update(cx, |view, _, cx| {
                view.ctx.no_terminal_sessions = true;
                cx.notify();
            })
            .unwrap();
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        settle(visual);
        assert!(!drawn(visual, "settings-agent-open-mode-tui"));
        assert!(!drawn(visual, "settings-agent-open-mode-chat"));
        assert!(!drawn(visual, "settings-agent-surface-claude-tui"));
        // The row stays, so search still finds it and shows why.
        assert!(drawn(visual, "settings-row-agent-open-mode"));
        // Inside a row there is nothing to choose either.
        press(visual, "settings-agent-header-claude");
        assert!(drawn(visual, "settings-agent-panel-claude"));
        assert!(!drawn(visual, "settings-agent-opens-claude-tui"));
    }

    #[gpui::test]
    fn only_one_agent_row_is_expanded_at_a_time(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude, Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(!drawn(visual, "settings-agent-panel-claude"));
        press(visual, "settings-agent-header-claude");
        assert!(drawn(visual, "settings-agent-panel-claude"));
        // Its fields: where it opens, permissions, command, arguments and the preview.
        for selector in [
            "settings-agent-opens-claude-chat",
            "settings-permission-claude",
            "settings-agent-command-claude",
            "settings-select-agent-model-claude",
            "settings-agent-effort-claude-high",
            "settings-agent-args-claude",
            "settings-agent-env-edit-claude",
            "settings-agent-preview-claude",
        ] {
            assert!(drawn(visual, selector), "{selector}");
        }
        press(visual, "settings-agent-header-codex");
        assert!(drawn(visual, "settings-agent-panel-codex"));
        assert!(!drawn(visual, "settings-agent-panel-claude"));
        // Pressing the open row closes it.
        press(visual, "settings-agent-header-codex");
        assert!(!drawn(visual, "settings-agent-panel-codex"));
    }

    #[gpui::test]
    fn picking_a_permission_writes_it_and_full_access_can_be_undone(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude, Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // The default every agent follows.
        press(visual, "settings-permission-global");
        for level in Permission::ALL {
            assert!(
                drawn(visual, format!("menu-action-permission:{}", level.id())),
                "{level:?}"
            );
        }
        press(visual, "menu-action-permission:edits");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.agents.permission.as_deref(), Some("edits"));
        assert_eq!(
            prefs.agent(Kind::Claude).permission,
            Permission::AcceptEdits
        );
        assert!(
            !drawn(visual, "settings-agent-permission-badge-claude"),
            "an agent that follows the default shows no badge"
        );

        // One agent set differently.
        press(visual, "settings-agent-header-claude");
        press(visual, "settings-permission-claude");
        assert!(drawn(visual, "menu-action-permission:inherit"));
        press(visual, "menu-action-permission:full");
        let prefs = prefs_of(window, visual);
        assert_eq!(
            prefs.agents.permissions.get("claude").map(String::as_str),
            Some("full")
        );
        assert_eq!(prefs.agent(Kind::Codex).permission, Permission::AcceptEdits);
        assert!(drawn(visual, "settings-agent-permission-badge-claude"));
        assert!(drawn(visual, "settings-agent-full-warning-claude"));
        assert!(drawn(visual, "settings-toast"));

        press(visual, "settings-toast-undo");
        let prefs = prefs_of(window, visual);
        assert!(prefs.agents.permissions.is_empty());
        assert!(!drawn(visual, "settings-toast"));
        assert!(!drawn(visual, "settings-agent-full-warning-claude"));

        // Back to the default from an override.
        press(visual, "settings-permission-claude");
        press(visual, "menu-action-permission:ask");
        assert_eq!(
            prefs_of(window, visual).agent(Kind::Claude).permission,
            Permission::Ask
        );
        press(visual, "settings-permission-claude");
        press(visual, "menu-action-permission:inherit");
        assert!(prefs_of(window, visual).agents.permissions.is_empty());
    }

    #[gpui::test]
    fn an_agent_hides_the_fields_and_levels_it_cannot_apply(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Cursor, Kind::Pi], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Pi has no permission mechanism; it does take a reasoning effort.
        press(visual, "settings-agent-header-pi");
        assert!(!drawn(visual, "settings-permission-pi"));
        assert!(drawn(visual, "settings-agent-effort-pi-default"));
        // Cursor approves on the client side only, and takes no effort of its own.
        press(visual, "settings-agent-header-cursor");
        assert!(!drawn(visual, "settings-agent-effort-cursor-default"));
        press(visual, "settings-permission-cursor");
        assert!(drawn(visual, "menu-action-permission:own"));
        assert!(drawn(visual, "menu-action-permission:auto"));
        assert!(drawn(visual, "menu-action-permission:full"));
        assert!(!drawn(visual, "menu-action-permission:ask"));
        assert!(!drawn(visual, "menu-action-permission:edits"));
    }

    #[gpui::test]
    fn a_known_flag_typed_into_the_arguments_moves_to_its_field(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        type_into(
            visual,
            "settings-agent-args-claude",
            "--model opus --permission-mode acceptEdits --verbose",
        );
        // The split result shows while typing; nothing is saved yet.
        visual.refresh().unwrap();
        assert!(drawn(visual, "settings-agent-chips-claude"));
        assert!(prefs_of(window, visual).agents.args.is_empty());

        visual.simulate_keystrokes("enter");
        settle(visual);
        let prefs = prefs_of(window, visual);
        assert_eq!(
            prefs.default_models.get("claude").map(String::as_str),
            Some("opus")
        );
        assert_eq!(
            prefs.agents.permissions.get("claude").map(String::as_str),
            Some("edits")
        );
        assert_eq!(
            prefs.agents.args.get("claude").map(String::as_str),
            Some("--verbose"),
            "other arguments pass through unchanged"
        );
        assert_eq!(prefs.agent(Kind::Claude).args, ["--verbose"]);
        // The notice says what moved and can take it back.
        assert!(drawn(visual, "settings-toast"));
        // Undo puts the moved flags back into the arguments and restores their fields;
        // the argument typed with them in the same edit is not discarded.
        press(visual, "settings-toast-undo");
        let prefs = prefs_of(window, visual);
        assert_eq!(
            prefs.agents.args.get("claude").map(String::as_str),
            Some("--verbose --model opus --permission-mode acceptEdits")
        );
        assert!(prefs.default_models.is_empty());
        assert!(prefs.agents.permissions.is_empty());
    }

    #[gpui::test]
    fn a_version_refresh_does_not_drop_a_command_being_saved(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        // A save's check and a background refresh start together; both must land.
        window
            .update(visual, |view, _, cx| {
                view.commit_agent_command(Kind::Claude, "claude-missing-build".into(), cx);
                view.inspect_agent(Kind::Claude, None, false, false, cx);
            })
            .unwrap();
        settle(visual);
        assert!(
            drawn(visual, "settings-agent-error-command-claude"),
            "the save's result is reported, not superseded by the refresh"
        );
        assert!(prefs_of(window, visual).binary_paths.is_empty());
    }

    #[gpui::test]
    fn arguments_that_cannot_be_split_keep_the_saved_value(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs
            .agents
            .args
            .insert("claude".into(), "--verbose".into());
        let window = agents_window(&[Kind::Claude], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        assert!(drawn(visual, "settings-agent-chips-claude"));
        type_into(visual, "settings-agent-args-claude", " \"unclosed");
        // Leaving the field saves it, and here refuses to.
        window
            .update(visual, |view, window, cx| window.focus(&view.focus, cx))
            .unwrap();
        settle(visual);
        assert!(drawn(visual, "settings-agent-error-args-claude"));
        assert!(!drawn(visual, "settings-saved"));
        let prefs = prefs_of(window, visual);
        assert_eq!(
            prefs.agents.args.get("claude").map(String::as_str),
            Some("--verbose")
        );
        assert_eq!(prefs.agent(Kind::Claude).args, ["--verbose"]);
    }

    #[gpui::test]
    fn valid_arguments_save_when_the_field_is_left(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-codex");
        type_into(visual, "settings-agent-args-codex", "--search \"a b\"");
        window
            .update(visual, |view, window, cx| window.focus(&view.focus, cx))
            .unwrap();
        settle(visual);
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.agent(Kind::Codex).args, ["--search", "a b"]);
        assert!(drawn(visual, "settings-saved"));
        assert!(!drawn(visual, "settings-toast"));
        // "Saved" goes away on its own.
        visual
            .executor()
            .advance_clock(std::time::Duration::from_secs(2));
        settle(visual);
        assert!(!drawn(visual, "settings-saved"));
    }

    #[gpui::test]
    fn text_typed_after_enter_saved_a_field_is_saved_when_the_field_is_left(
        cx: &mut TestAppContext,
    ) {
        let window = agents_window(&[Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-codex");
        type_into(visual, "settings-agent-args-codex", "--search");
        visual.simulate_keystrokes("enter");
        settle(visual);
        assert_eq!(
            prefs_of(window, visual).agent(Kind::Codex).args,
            ["--search"]
        );
        // Still in the field: what is typed next is not put back to the saved value.
        visual.simulate_input(" --verbose");
        settle(visual);
        window
            .update(visual, |view, window, cx| window.focus(&view.focus, cx))
            .unwrap();
        settle(visual);
        assert_eq!(
            prefs_of(window, visual).agent(Kind::Codex).args,
            ["--search", "--verbose"]
        );
    }

    #[gpui::test]
    fn a_later_change_retires_the_undo_of_an_earlier_notice(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs.default_provider = Some("claude".into());
        let window = agents_window(&[Kind::Claude, Kind::Codex], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-toggle-claude");
        assert!(drawn(visual, "settings-toast-undo"));
        // Undo would put back every preference from before the notice, this one too.
        press(visual, "settings-toggle-codex");
        settle(visual);
        assert!(drawn(visual, "settings-toast"));
        assert!(!drawn(visual, "settings-toast-undo"));
        assert!(!prefs_of(window, visual).agent(Kind::Codex).enabled);
    }

    #[gpui::test]
    fn an_error_under_a_field_goes_when_its_row_closes(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        type_into(visual, "settings-agent-args-claude", "\"unclosed");
        visual.simulate_keystrokes("enter");
        settle(visual);
        assert!(drawn(visual, "settings-agent-error-args-claude"));
        press(visual, "settings-agent-header-claude");
        press(visual, "settings-agent-header-claude");
        settle(visual);
        assert!(drawn(visual, "settings-agent-panel-claude"));
        assert!(!drawn(visual, "settings-agent-error-args-claude"));
    }

    #[gpui::test]
    fn turning_off_the_default_agent_clears_the_default(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs.default_provider = Some("claude".into());
        let window = agents_window(&[Kind::Claude, Kind::Codex], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        // Another agent goes off without touching the default.
        press(visual, "settings-toggle-codex");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.default_provider.as_deref(), Some("claude"));
        assert!(!prefs.agent(Kind::Codex).enabled);
        assert!(!drawn(visual, "settings-toast"));

        press(visual, "settings-toggle-claude");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.default_provider, None);
        assert!(!prefs.agent(Kind::Claude).enabled);
        assert!(drawn(visual, "settings-toast"));
        // Undo turns it back on and makes it the default again.
        press(visual, "settings-toast-undo");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.default_provider.as_deref(), Some("claude"));
        assert!(prefs.agent(Kind::Claude).enabled);
    }

    #[gpui::test]
    fn the_row_menu_sets_and_clears_the_default_and_resets_an_agent(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs.default_models.insert("claude".into(), "opus".into());
        prefs
            .agents
            .args
            .insert("claude".into(), "--verbose".into());
        let window = agents_window(&[Kind::Claude], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-menu-claude");
        for action in ["default:set", "check", "reveal", "copy", "reset", "docs"] {
            assert!(drawn(visual, format!("menu-action-{action}")), "{action}");
        }
        press(visual, "menu-action-default:set");
        assert_eq!(
            prefs_of(window, visual).default_provider.as_deref(),
            Some("claude")
        );
        press(visual, "settings-agent-menu-claude");
        assert!(!drawn(visual, "menu-action-default:set"));
        press(visual, "menu-action-default:clear");
        assert_eq!(prefs_of(window, visual).default_provider, None);

        press(visual, "settings-agent-menu-claude");
        press(visual, "menu-action-reset");
        let prefs = prefs_of(window, visual);
        assert!(prefs.default_models.is_empty());
        assert!(prefs.agents.args.is_empty());
        assert!(drawn(visual, "settings-toast"));

        // An agent that is not installed only links to its docs.
        press(visual, "settings-agent-menu-grok");
        assert!(drawn(visual, "menu-action-docs"));
        assert!(!drawn(visual, "menu-action-reset"));
    }

    #[gpui::test]
    fn a_row_set_differently_switches_the_open_mode_to_per_agent(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude, Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        // The option every agent already uses changes nothing.
        press(visual, "settings-agent-opens-claude-chat");
        assert_eq!(prefs_of(window, visual).open_mode(), OPEN_CHAT);
        assert!(!drawn(visual, "settings-toast"));

        press(visual, "settings-agent-opens-claude-tui");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.open_mode(), OPEN_PER_AGENT);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Tui);
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Chat);
        assert!(drawn(visual, "settings-toast"));
        assert!(drawn(visual, "settings-agent-surface-claude-tui"));

        press(visual, "settings-toast-undo");
        let prefs = prefs_of(window, visual);
        assert_eq!(prefs.open_mode(), OPEN_CHAT);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Chat);
    }

    #[gpui::test]
    fn the_command_is_checked_before_it_replaces_the_saved_one(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        // No such command on the PATH: refused, with the reason, without starting anything.
        type_into(visual, "settings-agent-command-claude", "claude-nightly");
        visual.simulate_keystrokes("enter");
        settle(visual);
        assert!(drawn(visual, "settings-agent-error-command-claude"));
        assert!(prefs_of(window, visual).binary_paths.is_empty());
        assert_eq!(prefs_of(window, visual).agent(Kind::Claude).command, None);
    }

    #[gpui::test]
    fn the_model_can_be_the_agents_default_or_a_typed_name(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs.default_models.insert("claude".into(), "opus".into());
        let window = agents_window(&[Kind::Claude], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        // No catalog is loaded: "Agent default" is still there to pick.
        press(visual, "settings-select-agent-model-claude");
        press(visual, "settings-option-agent-model-claude-0");
        assert!(prefs_of(window, visual).default_models.is_empty());
        assert_eq!(prefs_of(window, visual).agent(Kind::Claude).model, None);

        // "Custom model…" is the last option; the name is passed as typed.
        press(visual, "settings-select-agent-model-claude");
        press(visual, "settings-option-agent-model-claude-1");
        assert!(drawn(visual, "settings-agent-model-claude"));
        type_into(visual, "settings-agent-model-claude", "my-tuned-model");
        visual.simulate_keystrokes("enter");
        settle(visual);
        assert_eq!(
            prefs_of(window, visual)
                .agent(Kind::Claude)
                .model
                .as_deref(),
            Some("my-tuned-model")
        );
    }

    #[gpui::test]
    fn an_invalid_variable_line_keeps_the_saved_environment(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        let saved = concat!(
            "HTTP_PROXY=http://127.0.0.1:9\n",
            "SERVICE_TOKEN=",
            "not-a-real-token"
        );
        prefs.agents.env.insert("claude".into(), saved.into());
        let window = agents_window(&[Kind::Claude], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        // Read view: one row per variable; only the secret one can be revealed.
        assert!(drawn(visual, "settings-agent-env-table-claude"));
        assert!(!drawn(visual, "settings-agent-env-reveal-claude-0"));
        assert!(drawn(visual, "settings-agent-env-reveal-claude-1"));

        press(visual, "settings-agent-env-edit-claude");
        assert!(drawn(visual, "settings-agent-env-claude"));
        visual.simulate_keystrokes("secondary-a");
        visual.simulate_input("this is not a pair");
        press(visual, "settings-agent-env-done-claude");
        settle(visual);
        assert!(drawn(visual, "settings-agent-error-env-claude"));
        assert_eq!(
            prefs_of(window, visual)
                .agents
                .env
                .get("claude")
                .map(String::as_str),
            Some(saved)
        );

        // A valid edit saves and goes back to the table.
        press(visual, "settings-agent-env-claude");
        visual.simulate_keystrokes("secondary-a");
        visual.simulate_input("RUST_LOG=debug");
        press(visual, "settings-agent-env-done-claude");
        settle(visual);
        let prefs = prefs_of(window, visual);
        assert_eq!(
            prefs.agent(Kind::Claude).env,
            [("RUST_LOG".to_string(), "debug".to_string())]
        );
        assert!(drawn(visual, "settings-agent-env-table-claude"));
        assert!(!drawn(visual, "settings-agent-error-env-claude"));
    }

    #[gpui::test]
    fn the_launch_command_follows_the_agents_open_mode(cx: &mut TestAppContext) {
        let mut prefs = agent_prefs(OPEN_CHAT);
        prefs.agents.permission = Some(Permission::AcceptEdits.id().into());
        prefs.default_models.insert("claude".into(), "opus".into());
        prefs
            .agents
            .args
            .insert("claude".into(), "--verbose".into());
        prefs
            .agents
            .env
            .insert("claude".into(), "HTTP_PROXY=http://127.0.0.1:9".into());
        let window = agents_window(&[Kind::Claude], prefs, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        press(visual, "settings-agent-header-claude");
        let copied = |visual: &mut VisualTestContext| {
            press(visual, "settings-agent-copy-launch-claude");
            visual
                .read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap_or_default()
        };
        assert_eq!(
            copied(visual),
            "HTTP_PROXY=http://127.0.0.1:9 claude --permission-mode acceptEdits --model opus --verbose"
        );
        // A terminal session passes the flags `tui_launch::build` does, in its order.
        press(visual, "settings-agent-open-mode-tui");
        assert_eq!(
            copied(visual),
            "HTTP_PROXY=http://127.0.0.1:9 claude --model opus --permission-mode acceptEdits --verbose"
        );
    }

    #[gpui::test]
    fn an_agent_that_is_not_installed_shows_how_to_install_it_and_a_path_field(
        cx: &mut TestAppContext,
    ) {
        let window = agents_window(&[Kind::Claude], agent_prefs(OPEN_PER_AGENT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(drawn(visual, "settings-agent-install-grok"));
        assert!(
            !drawn(visual, "settings-toggle-grok"),
            "only an installed agent can be turned on or off"
        );
        press(visual, "settings-agent-header-grok");
        assert!(drawn(visual, "settings-agent-panel-grok"));
        assert!(drawn(visual, "settings-agent-command-grok"));
        for selector in [
            "settings-permission-grok",
            "settings-agent-args-grok",
            "settings-agent-opens-grok-chat",
            "settings-agent-preview-grok",
        ] {
            assert!(!drawn(visual, selector), "{selector}");
        }
    }

    #[gpui::test]
    fn detection_shows_a_skeleton_while_it_runs(cx: &mut TestAppContext) {
        let window = agents_window(&[], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(!drawn(visual, "settings-agent-detecting"));
        window
            .update(visual, |view, _, cx| {
                view.discovery_loading = true;
                cx.notify();
            })
            .unwrap();
        settle(visual);
        assert!(drawn(visual, "settings-agent-detecting"));
    }

    #[gpui::test]
    fn search_lands_on_an_agents_row_and_opens_it(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        let result = model::search("codex", "Wayland")
            .into_iter()
            .find(|result| result.setting == Some("settings-agent-codex"))
            .expect("the Codex row is a search result");
        window
            .update(visual, |view, _, cx| view.go(result, cx))
            .unwrap();
        settle(visual);
        assert!(drawn(visual, "settings-agent-codex"));
        assert!(drawn(visual, "settings-agent-panel-codex"));
    }

    #[gpui::test]
    fn an_update_found_by_the_shell_shows_on_the_agents_row(cx: &mut TestAppContext) {
        let window = agents_window(&[Kind::Claude, Kind::Codex], agent_prefs(OPEN_CHAT), cx);
        window
            .update(cx, |view, _, cx| {
                view.ctx.agent_updates = vec![AgentUpdate {
                    kind: Kind::Codex,
                    latest: "9.9.9".into(),
                    state: AgentUpdateState::Available,
                }];
                cx.notify();
            })
            .unwrap();
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let _subscription = cx.update(|cx| {
            let events = events.clone();
            let view = window.root(cx).unwrap();
            cx.subscribe(&view, move |_, event: &SettingsEvent, _| {
                if let SettingsEvent::UpdateAgent(kind) = event {
                    events.borrow_mut().push(*kind);
                }
            })
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        settle(visual);
        assert!(drawn(visual, "settings-agent-update-codex"));
        assert!(!drawn(visual, "settings-agent-update-claude"));
        // The button hands the update to the existing flow.
        press(visual, "settings-agent-update-codex");
        assert_eq!(*events.borrow(), [Kind::Codex]);
    }
}
