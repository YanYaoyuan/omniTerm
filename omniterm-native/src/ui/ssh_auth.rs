// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Standalone askpass window. It runs in OpenSSH's helper process, keeping
//! authentication prompts and keychain waits out of the main terminal window.
use crate::core::ssh_auth::{save, valid_secret};
use eframe::egui;
use std::sync::{Arc, Mutex};

struct PasswordWindow {
    prompt: String,
    scope: String,
    first: bool,
    confirm: bool,
    value: String,
    remember: bool,
    error: Option<String>,
    focused: bool,
    result: Arc<Mutex<Option<String>>>,
}

impl PasswordWindow {
    fn submit(&mut self, ctx: &egui::Context) {
        if !self.confirm && !valid_secret(&self.value) {
            self.error = Some("请输入有效内容（不能包含换行）。".into());
            return;
        }
        if self.first && self.remember && save(&self.scope, &self.prompt, &self.value).is_err() {
            self.error = Some("系统钥匙串不可用或未解锁。请解锁钥匙串，或取消勾选“记住”以仅用于本次登录。".into());
            return;
        }
        let answer = if self.confirm { "yes".into() } else { std::mem::take(&mut self.value) };
        *self.result.lock().unwrap_or_else(|p| p.into_inner()) = Some(answer);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for PasswordWindow {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ui.painter().rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
        ui.scope_builder(egui::UiBuilder::new().max_rect(ui.max_rect().shrink(16.0)), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading(if self.confirm {
                    "SSH 主机身份确认"
                } else if self.first {
                    "SSH 登录密码"
                } else {
                    "SSH 验证（仅本次）"
                });
                ui.add_space(8.0);
                ui.label(&self.prompt);
                ui.add_space(10.0);
                if !self.confirm {
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut self.value)
                            .password(true)
                            .char_limit(4096)
                            .desired_width(f32::INFINITY),
                    );
                    if !self.focused {
                        input.request_focus();
                        self.focused = true;
                    }
                    if self.first {
                        ui.checkbox(&mut self.remember, "记住首次密码（保存在系统钥匙串）");
                    } else {
                        ui.label("验证码、密钥口令和后续认证内容不会保存。输入内容仅交给当前 SSH 认证过程。");
                    }
                }
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 50, 50), error);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button(if self.confirm { "确认此主机并继续" } else { "继续登录" }).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.submit(&ctx);
                    }
                });
            });
        });
    }
}

pub fn show(scope: &str, prompt: &str, first: bool, confirm: bool) -> Option<String> {
    let result = Arc::new(Mutex::new(None));
    let output = result.clone();
    let window = PasswordWindow {
        prompt: prompt.to_string(),
        scope: scope.to_string(),
        first,
        confirm,
        value: String::new(),
        remember: first,
        error: None,
        focused: false,
        result: result.clone(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 340.0])
            .with_min_inner_size([440.0, 300.0])
            .with_title("omniTerm · SSH 认证"),
        ..Default::default()
    };
    if eframe::run_native(
        "omniTerm SSH",
        options,
        Box::new(move |cc| {
            crate::ui::style::install_fonts(&cc.egui_ctx);
            Ok(Box::new(window))
        }),
    )
    .is_err()
    {
        return None;
    }
    output.lock().unwrap_or_else(|p| p.into_inner()).take()
}
