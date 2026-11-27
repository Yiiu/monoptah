use crate::{
    settings::{ModelChoice, SettingsView, widgets::note},
    theme::*,
};
use gpui::{AnyElement, Context, SharedString, Window, prelude::*};
use monocode_core::provider::Kind;

impl SettingsView {
    /// Accounts, usage and privacy, the Agent CLIs groups (`pages/agents.rs`), background
    /// tasks and advanced switches.
    pub(super) fn providers_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = self.prefs.clone();
        let accounts = if let Some(center) = crate::accounts::get(cx) {
            center.update(cx, |center, cx| {
                center.mask_emails = p.mask_emails;
                center.show_remaining = p.show_remaining_usage;
                for kind in monocode_core::accounts::PROVIDERS {
                    center.load_provider(kind, false, cx);
                }
                center.settings_rows(cx)
            })
        } else {
            vec![note("Provider accounts load with the workspace.")]
        };
        let usage = vec![
            self.row(
                Some("show-remaining-usage"),
                true,
                "Show remaining usage",
                Some("Show remaining capacity in usage meters.".into()),
                vec![self.toggle(
                    "show-remaining-usage",
                    "Show remaining usage",
                    p.show_remaining_usage,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.show_remaining_usage = on),
                )],
            ),
            self.row(
                Some("mask-emails"),
                false,
                "Mask account emails",
                Some("Hide email addresses until clicked.".into()),
                vec![self.toggle(
                    "mask-emails",
                    "Mask account emails",
                    p.mask_emails,
                    false,
                    cx,
                    |this, on, cx| this.update(cx, |p| p.mask_emails = on),
                )],
            ),
        ];
        let binaries = self.discovery.binaries.clone();
        let agents = self.agent_groups(window, cx);
        // Any installed harness that can write one-off text, with the models
        // its sessions have listed; the harness default when none are loaded.
        let mut background = vec![(
            "auto".to_owned(),
            SharedString::from("Automatic"),
            None::<AnyElement>,
        )];
        for kind in Kind::ALL {
            let installed = binaries.iter().any(|(k, p)| *k == kind && p.is_some());
            if !installed || !monocode_core::session_title::supports(kind) {
                continue;
            }
            match self.ctx.catalogs.get(kind.id()).filter(|m| !m.is_empty()) {
                Some(models) => background.extend(models.iter().map(|(id, name)| {
                    (
                        format!("{}:{id}", kind.id()),
                        SharedString::from(format!("{} · {name}", kind.label())),
                        Some(provider_icon(kind.id(), 14.).into_any_element()),
                    )
                })),
                None => background.push((
                    format!("{}:", kind.id()),
                    SharedString::from(format!("{} · Default model", kind.label())),
                    Some(provider_icon(kind.id(), 14.).into_any_element()),
                )),
            }
        }
        let background_value = p
            .background_model
            .as_ref()
            .map(|c| format!("{}:{}", c.provider, c.model.as_deref().unwrap_or_default()))
            .unwrap_or_else(|| "auto".into());
        if !background.iter().any(|(v, _, _)| *v == background_value) {
            // A saved pick whose catalog is not loaded keeps its own label.
            let choice = p.background_model.clone().unwrap_or(ModelChoice {
                provider: String::new(),
                model: None,
            });
            background.push((
                background_value.clone(),
                SharedString::from(format!(
                    "{} · {}",
                    Kind::from_id(&choice.provider).label(),
                    choice.model.as_deref().unwrap_or("Default model")
                )),
                Some(provider_icon(&choice.provider, 14.).into_any_element()),
            ));
        }
        let background = vec![self.row(
            Some("background-model"),
            true,
            "Background model",
            Some("Writes session titles, progress summaries, worktree branch names, commit messages and pull requests. Automatic uses each session's provider and its quick model.".into()),
            vec![self.select(
                "background-model",
                "Background model",
                &background_value,
                background,
                Some(240.),
                cx,
                |this, value, cx| {
                    this.update(cx, |p| {
                        p.background_model = value.split_once(':').map(|(provider, model)| {
                            ModelChoice {
                                provider: provider.into(),
                                model: (!model.is_empty()).then(|| model.into()),
                            }
                        });
                    })
                },
            )],
        )];
        let advanced=vec![
            self.row(Some("idle-agents"),true,"Keep idle agents ready",Some("Keep up to this many idle agents running for faster follow-ups. Uses additional memory.".into()),vec![self.segmented("idle-agents","Keep idle agents ready",&p.idle_agents.to_string(),&[("0","None"),("1","1"),("2","2"),("3","3"),("5","5")],cx,|this,value,cx|if let Ok(value)=value.parse(){this.update(cx,|p|p.idle_agents=value)})]),
            self.row(Some("claude-hooks"),false,"Claude Code hooks",Some("Run your configured Claude Code hooks. Changes apply on the next turn.".into()),vec![self.toggle("claude-hooks","Claude Code hooks",p.claude_hooks,false,cx,|this,on,cx|this.update(cx,|p|p.claude_hooks=on))]),
            self.row(Some("claude-entrypoint"),false,"Identify as Monoptah to Claude (experimental)",Some("Report Monoptah as the client instead of the default sdk-cli, which community tests show uses the 5-hour limit up to ~2x faster. Anthropic hasn't confirmed this or whether relabeling is allowed. Applies to new sessions.".into()),vec![self.toggle("claude-entrypoint","Identify as Monoptah to Claude",p.claude_entrypoint,false,cx,|this,on,cx|this.update(cx,|p|p.claude_entrypoint=on))])];
        let mut page = vec![
            self.group(
                Some("provider-accounts"),
                true,
                "Accounts",
                Some("Manage sign-ins and choose accounts for each project.".into()),
                Some(self.refresh_button("refresh-provider-accounts", "Refresh", cx)),
                accounts,
            ),
            self.group(None, false, "Usage and privacy", None, None, usage),
        ];
        page.extend(agents);
        page.push(self.group(
            None,
            false,
            "Background tasks",
            Some("Small jobs Monoptah runs with a model on your behalf.".into()),
            None,
            background,
        ));
        page.push(self.group(None, false, "Advanced", None, None, advanced));
        page
    }
}
