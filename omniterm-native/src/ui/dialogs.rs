// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
//! Settings, connections, deleting a connection and importing hosts from `~/.ssh/config`.

use super::style::{self, icon, icon_button, percent_slider, segmented, toggle};
use super::{App, Dialog, Intent};
use crate::core::profile::{Forward, Kind, Profile};
use crate::core::settings::{BackgroundMode, CursorStyle, FontFamily, ImageFit, LINE_HEIGHTS, LocalShell, MAX_FONT, MIN_FONT, SCROLLBACKS, Settings};
use crate::core::ssh_config::Host;
use crate::icons;
use crate::theme::THEMES;
use eframe::egui::{self, Align, Color32, Id, Layout, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub struct ConnectionForm {
    pub kind: Kind,
    pub editing: Option<Profile>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub key_path: String,
    pub proxy_jump: String,
    pub device: String,
    pub baud: String,
    pub forward: Forward,
    pub bind_port: String,
    pub target_host: String,
    pub target_port: String,
    pub persist: bool,
    pub error: Option<String>,
}

impl ConnectionForm {
    pub fn new(kind: Kind, profile: Option<Profile>) -> Self {
        let value = |field: &Option<String>| field.clone().unwrap_or_default();
        let number = |field: Option<u16>| field.map(|n| n.to_string()).unwrap_or_default();
        let p = profile.clone().unwrap_or_default();
        Self {
            kind,
            name: value(&p.name),
            host: value(&p.host),
            port: number(p.port),
            username: value(&p.username),
            key_path: value(&p.key_path),
            proxy_jump: value(&p.proxy_jump),
            device: value(&p.device),
            baud: p.baud.unwrap_or(115_200).to_string(),
            forward: p.forward_type.unwrap_or(Forward::L),
            bind_port: number(p.bind_port),
            target_host: p.target_host.clone().unwrap_or_else(|| "localhost".into()),
            target_port: number(p.target_port),
            persist: true,
            error: None,
            editing: profile,
        }
    }

    /// The profile the form describes. Numbers are checked here; the rest by `validate`.
    fn profile(&self) -> Result<Profile, String> {
        let optional = |text: &str| Some(text.trim().to_string()).filter(|text| !text.is_empty());
        let port = |text: &str, label: &str| -> Result<Option<u16>, String> {
            match text.trim() {
                "" => Ok(None),
                value => value.parse::<u16>().ok().filter(|port| *port > 0).map(Some).ok_or_else(|| format!("{label}必须是 1–65535 的整数。")),
            }
        };
        let target = if self.kind == Kind::Serial { self.device.trim() } else { self.host.trim() };
        let label = super::sidebar::connection_label(self.kind);
        let mut profile = Profile {
            id: self.editing.as_ref().and_then(|profile| profile.id.clone()),
            kind: Some(self.kind),
            name: Some(optional(&self.name).unwrap_or_else(|| format!("{label} · {target}"))),
            ..Profile::default()
        };
        if self.kind == Kind::Serial {
            profile.device = optional(&self.device);
            profile.baud = Some(self.baud.trim().parse().map_err(|_| "波特率必须是 1–4000000 的整数。".to_string())?);
        } else {
            profile.host = optional(&self.host);
            profile.port = port(&self.port, "SSH 端口")?;
            profile.username = optional(&self.username);
            profile.key_path = optional(&self.key_path);
            profile.proxy_jump = optional(&self.proxy_jump);
            // The imported label describes the host it came from.
            if self.editing.as_ref().is_some_and(|old| old.host == profile.host) {
                profile.summary = self.editing.as_ref().and_then(|old| old.summary.clone());
            }
        }
        if self.kind == Kind::Tunnel {
            profile.forward_type = Some(self.forward);
            profile.bind_port = port(&self.bind_port, "监听端口")?;
            if self.forward != Forward::D {
                profile.target_host = optional(&self.target_host);
                profile.target_port = port(&self.target_port, "目标端口")?;
            }
        }
        profile.validate(true)
    }
}

pub struct ImportState {
    pub path: PathBuf,
    pub hosts: Vec<Host>,
    pub selected: BTreeSet<String>,
    pub startup: bool,
    pub error: Option<String>,
}

impl ImportState {
    pub fn new(path: PathBuf, hosts: Vec<Host>, startup: bool) -> Self {
        let selected = hosts.iter().map(|host| host.alias.clone()).collect();
        Self { path, hosts, selected, startup, error: None }
    }
}

/// A modal dialog with the desktop app's heading: an icon tile, a title and a subtitle.
fn modal(ctx: &egui::Context, id: &str, width: f32, glyph: &str, title: &str, subtitle: &str, body: impl FnOnce(&mut Ui)) -> bool {
    let palette = style::palette_of(ctx);
    let mut close = false;
    let response = egui::Modal::new(Id::unique(id))
        .backdrop_color(Color32::from_black_alpha(140))
        .frame(egui::Frame::new().fill(palette.surface()).stroke(Stroke::new(1.0, palette.border_strong)).corner_radius(12).inner_margin(22))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                let (tile, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
                ui.painter().rect_filled(tile, 10.0, style::alpha(palette.tokens.accent, 0.12));
                ui.painter().text(tile.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::new(22.0, egui::FontFamily::Name(style::ICONS.into())), palette.accent());
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).size(16.0).strong().color(palette.text()));
                    ui.label(RichText::new(subtitle).size(11.0).color(palette.muted()));
                });
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| close = icon_button(ui, icons::X, "关闭", true).clicked());
            });
            ui.add_space(14.0);
            body(ui);
        });
    close || response.should_close()
}

fn setting_row(ui: &mut Ui, title: &str, detail: &str, control: impl FnOnce(&mut Ui)) {
    let palette = style::palette(ui);
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        ui.vertical(|ui| {
            ui.set_max_width(300.0);
            ui.label(RichText::new(title).size(12.0).color(palette.text()));
            ui.add(egui::Label::new(RichText::new(detail).size(10.5).color(palette.muted())).wrap());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
    ui.add_space(2.0);
}

fn section(ui: &mut Ui, title: &str) {
    let palette = style::palette(ui);
    ui.add_space(10.0);
    ui.label(RichText::new(title).size(10.5).strong().color(palette.accent()));
    ui.add_space(4.0);
}

fn shell_settings(ui: &mut Ui, shell: &mut LocalShell) {
    let palette = style::palette(ui);
    section(ui, "本地 Shell");
    ui.add(egui::Label::new(RichText::new("用于新建、分屏及恢复的本地终端；修改后已打开的会话继续运行。").size(10.5).color(palette.muted())).wrap());
    ui.label(RichText::new("程序名称或完整路径（不加外层引号）").size(11.0));
    ui.add(egui::TextEdit::singleline(&mut shell.program).hint_text(if cfg!(windows) { "留空自动选择；例如 pwsh.exe、cmd.exe、wsl.exe" } else { "留空使用系统默认；例如 /bin/zsh、/bin/bash" }).desired_width(ui.available_width()));
    ui.add_space(4.0);
    ui.label(RichText::new("启动参数（按顺序，每行是一个完整参数）").size(11.0));
    let mut remove = None;
    for (index, arg) in shell.args.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 60.0).max(40.0);
            ui.add(egui::TextEdit::singleline(arg).id_salt(("shell-argument", index)).desired_width(width).hint_text("例如 -l；含空格的参数无需再加引号"));
            if style::ghost_button(ui, "移除").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        shell.args.remove(index);
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(shell.args.len() < 64, egui::Button::new("添加参数")).clicked() {
            shell.args.push(String::new());
        }
        if style::ghost_button(ui, "恢复系统默认").clicked() {
            *shell = LocalShell::default();
        }
    });
    ui.add(egui::Label::new(RichText::new("程序和参数都留空时沿用系统默认；添加参数后替换默认参数。参数按原样传给程序，不会拼成命令行。").size(10.5).color(palette.muted())).wrap());
    ui.add_space(6.0);
}

pub fn show(app: &mut App, ctx: &egui::Context, intents: &mut Vec<Intent>) {
    let Some(dialog) = app.dialog.as_mut() else { return };
    let close = match dialog {
        Dialog::Settings => settings(app, ctx, intents),
        Dialog::Connection(form) => connection(form, ctx, intents),
        Dialog::Delete { profile, error } => delete(profile, error.as_deref(), ctx, intents),
        Dialog::SshImport(state) => import(state, ctx, intents),
    };
    if close {
        intents.push(Intent::CloseDialog);
    }
}

fn settings(app: &mut App, ctx: &egui::Context, intents: &mut Vec<Intent>) -> bool {
    let palette = style::palette_of(ctx);
    let mac = app.mac;
    let window_transparent = app.window_transparent;
    let settings = &mut app.settings;
    let mut done = false;
    let closed = modal(ctx, "settings", 560.0, icons::SLIDERS_HORIZONTAL, "终端设置", "让工作区更适合你的习惯", |ui| {
        egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).show(ui, |ui| {
            shell_settings(ui, &mut settings.local_shell);
            section(ui, "外观");
            egui::Grid::new("themes").spacing(Vec2::splat(8.0)).show(ui, |ui| {
                for (index, theme) in THEMES.iter().enumerate() {
                    let selected = settings.theme == theme.id;
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(128.0, 58.0), Sense::click());
                    ui.painter().rect(rect, 8.0, style::color(theme.background), Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { palette.accent() } else { palette.border_strong }), egui::StrokeKind::Inside);
                    for (dot, color) in [1usize, 2, 3, 4, 5, 6].iter().enumerate() {
                        ui.painter().circle_filled(rect.left_top() + Vec2::new(14.0 + dot as f32 * 12.0, 18.0), 4.0, style::color(theme.ansi[*color]));
                    }
                    ui.painter().text(rect.left_bottom() + Vec2::new(10.0, -12.0), egui::Align2::LEFT_CENTER, theme.name, egui::FontId::proportional(11.0), style::color(theme.foreground));
                    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        settings.theme = theme.id.into();
                    }
                    if index % 4 == 3 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(8.0);
            setting_row(ui, "窗口背景", "半透明和透明会透出桌面；Linux 需要桌面合成器", |ui| {
                segmented(ui, &mut settings.background_mode, &BackgroundMode::ALL.map(|mode| (mode, mode.label())));
            });
            if settings.background_mode == BackgroundMode::Translucent {
                setting_row(ui, "背景不透明度", "数值越低，透出的桌面越多", |ui| {
                    percent_slider(ui, &mut settings.background_opacity, 10.0, 95.0);
                });
            }
            if settings.transparent_window() != window_transparent {
                egui::Frame::new().fill(style::alpha(palette.tokens.warning, 0.1)).corner_radius(6).inner_margin(10).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(format!("切换{}需要重启应用，当前会话会结束。", if settings.transparent_window() { "到透明窗口" } else { "回不透明窗口" })).size(11.0)).wrap());
                        if style::secondary_button(ui, "立即重启").clicked() {
                            intents.push(Intent::Relaunch);
                        }
                    });
                });
            }
            let image_detail = settings.background_image.as_ref().map(|image| image.name.clone()).unwrap_or_else(|| "支持 PNG、JPEG、WebP、GIF、BMP，最大 32 MB".into());
            setting_row(ui, "背景图片", &image_detail, |ui| {
                if settings.background_image.is_some() && style::ghost_button(ui, "移除").clicked() {
                    intents.push(Intent::ClearBackground);
                }
                if style::secondary_button(ui, "选择图片…").clicked() {
                    intents.push(Intent::ChooseBackground);
                }
            });
            if settings.background_image.is_some() {
                setting_row(ui, "图片强度", "降低后文字更清晰", |ui| {
                    percent_slider(ui, &mut settings.background_image_opacity, 5.0, 100.0);
                });
                setting_row(ui, "图片填充", "图片在窗口中的显示方式", |ui| {
                    egui::ComboBox::from_id_salt("fit").selected_text(settings.background_image_fit.label()).show_ui(ui, |ui| {
                        for fit in ImageFit::ALL {
                            ui.selectable_value(&mut settings.background_image_fit, fit, fit.label());
                        }
                    });
                });
            }
            if settings.background_mode != BackgroundMode::Opaque || settings.background_image.is_some() {
                setting_row(ui, "面板与终端不透明度", "所有面板和终端使用同一层底色，透明程度一致", |ui| {
                    percent_slider(ui, &mut settings.surface_opacity, 0.0, 95.0);
                });
            }
            setting_row(ui, "字体", "未安装的字体会回退到系统等宽字体", |ui| {
                egui::ComboBox::from_id_salt("font").selected_text(settings.font_family.label()).show_ui(ui, |ui| {
                    for family in FontFamily::ALL {
                        ui.selectable_value(&mut settings.font_family, family, family.label());
                    }
                });
            });
            setting_row(ui, "字体大小", "应用到所有终端和 Agent 会话", |ui| {
                if ui.add_enabled(settings.font_size < MAX_FONT, egui::Button::new("+")).clicked() {
                    settings.font_size += 1.0;
                }
                ui.label(RichText::new(format!("{} px", settings.font_size)).monospace());
                if ui.add_enabled(settings.font_size > MIN_FONT, egui::Button::new("−")).clicked() {
                    settings.font_size -= 1.0;
                }
            });
            setting_row(ui, "行高", "行与行之间的距离", |ui| {
                egui::ComboBox::from_id_salt("line-height").selected_text(settings.line_height.to_string()).show_ui(ui, |ui| {
                    for height in LINE_HEIGHTS {
                        ui.selectable_value(&mut settings.line_height, height, height.to_string());
                    }
                });
            });
            preview(ui, settings);
            setting_row(ui, "光标样式", "输入位置的光标形状", |ui| {
                let label = |style: CursorStyle| match style {
                    CursorStyle::Bar => "竖线",
                    CursorStyle::Block => "方块",
                    CursorStyle::Underline => "下划线",
                };
                egui::ComboBox::from_id_salt("cursor").selected_text(label(settings.cursor_style)).show_ui(ui, |ui| {
                    for style in [CursorStyle::Bar, CursorStyle::Block, CursorStyle::Underline] {
                        ui.selectable_value(&mut settings.cursor_style, style, label(style));
                    }
                });
            });
            setting_row(ui, "光标闪烁", "在输入位置显示闪烁光标", |ui| {
                toggle(ui, &mut settings.cursor_blink, "光标闪烁");
            });
            setting_row(ui, "淡化非活动窗格", "分屏时突出当前焦点窗格", |ui| {
                toggle(ui, &mut settings.dim_inactive, "淡化非活动窗格");
            });
            section(ui, "行为与性能");
            setting_row(ui, "选中即复制", "选择文本后自动写入剪贴板", |ui| {
                toggle(ui, &mut settings.copy_on_select, "选中即复制");
            });
            if mac {
                setting_row(ui, "Option 作为 Meta 键", "用于 Emacs、readline 的 Alt 组合键", |ui| {
                    toggle(ui, &mut settings.option_as_meta, "Option 作为 Meta 键");
                });
            }
            setting_row(ui, "启动时恢复工作区", "重新打开本地标签、分屏布局和各窗格目录，不会重新执行命令", |ui| {
                toggle(ui, &mut settings.restore_workspace, "启动时恢复工作区");
            });
            setting_row(ui, "SSH 文件面板与主机监控", "新建 SSH 连接时共享登录、显示 SFTP 文件和远程主机指标；对已打开的连接不生效", |ui| {
                toggle(ui, &mut settings.remote_panels, "SSH 文件面板与主机监控");
            });
            setting_row(ui, "启动时检查 SSH 配置", "~/.ssh/config 中出现新主机时询问是否导入", |ui| {
                toggle(ui, &mut settings.ssh_config_prompt, "启动时检查 SSH 配置");
                if style::secondary_button(ui, "立即导入…").clicked() {
                    intents.push(Intent::OpenSshImport);
                }
            });
            setting_row(ui, "回滚缓冲区", "保留每个终端的历史输出行数", |ui| {
                egui::ComboBox::from_id_salt("scrollback").selected_text(format!("{} 行", group(settings.scrollback))).show_ui(ui, |ui| {
                    for lines in SCROLLBACKS {
                        ui.selectable_value(&mut settings.scrollback, lines, format!("{} 行", group(lines)));
                    }
                });
            });
            section(ui, "键盘快捷键");
            let mut shortcuts = super::shortcuts::all(mac);
            shortcuts.push(("切换标签", "Ctrl+Tab".into()));
            shortcuts.push(("跳转到第 N 个标签", if mac { "⌘1…9".into() } else { "Alt+1…9".into() }));
            shortcuts.push(("打开链接", format!("{}+点击", if mac { "⌘" } else { "Ctrl" })));
            for (label, keys) in shortcuts {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(11.5).color(palette.text_soft()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| style::kbd(ui, &keys));
                });
            }
            ui.add_space(12.0);
            egui::Frame::new().fill(style::alpha(palette.tokens.ink, 0.03)).corner_radius(8).inner_margin(12).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(icon(icons::TERMINAL_WINDOW, 20.0).color(palette.accent()));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("omniTerm  {}", env!("CARGO_PKG_VERSION"))).strong().color(palette.text()));
                        ui.label(RichText::new("你的终端，你的 Agent。").size(10.5).color(palette.muted()));
                    });
                });
            });
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if style::ghost_button(ui, "恢复默认").clicked() {
                if settings.background_image.is_some() {
                    intents.push(Intent::ClearBackground);
                }
                *settings = Settings { background_image: settings.background_image.clone(), ..Settings::default() };
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| done = style::primary_button(ui, &format!("完成  {}", icons::CHECK)).clicked());
        });
    });
    closed || done
}

fn group(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

fn preview(ui: &mut Ui, settings: &Settings) {
    let theme = crate::theme::by_id(&settings.theme);
    let font = egui::FontId::monospace(settings.font_size);
    egui::Frame::new().fill(style::color(theme.background)).corner_radius(8).inner_margin(14).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = settings.font_size * (settings.line_height - 1.0);
        let text = |text: &str, color| RichText::new(text).font(font.clone()).color(style::color(color));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(text("omniTerm ~/workspace $ ", theme.foreground));
            ui.label(text("git", theme.ansi[2]));
            ui.label(text(" status", theme.foreground));
            let (cursor, _) = ui.allocate_exact_size(Vec2::new(2.0, settings.font_size), Sense::hover());
            ui.painter().rect_filled(cursor, 0.0, style::color(theme.cursor));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(text("中文对齐", theme.ansi[4]));
            ui.label(text(" → ", theme.foreground));
            ui.label(text("✓ 0 errors", theme.ansi[3]));
        });
    });
    ui.add_space(6.0);
}

fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str, width: f32) {
    let palette = style::palette(ui);
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.label(RichText::new(label).size(11.0).color(palette.muted()));
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(width).margin(Vec2::new(8.0, 6.0)));
    });
}

fn connection(form: &mut ConnectionForm, ctx: &egui::Context, intents: &mut Vec<Intent>) -> bool {
    let palette = style::palette_of(ctx);
    let title = if form.editing.is_some() { "编辑连接" } else { "新建连接" };
    modal(ctx, "connection", 470.0, icons::PLUGS_CONNECTED, title, "配置并管理你的终端连接", |ui| {
        let kinds = Kind::CONNECTIONS.map(|kind| (kind, super::sidebar::connection_label(kind)));
        segmented(ui, &mut form.kind, &kinds);
        ui.add_space(6.0);
        let description = match form.kind {
            Kind::Ssh => "通过 SSH 安全连接到远程主机。",
            Kind::Sftp => "打开交互式 SFTP 会话，上传和下载文件。",
            Kind::Serial => "连接本地串口设备并打开交互控制台。",
            _ => "建立 SSH 隧道，安全转发本地或远程端口。",
        };
        ui.label(RichText::new(description).size(11.0).color(palette.muted()));
        ui.add_space(8.0);
        let full = ui.available_width();
        let host_width = full - 120.0;
        field(ui, "连接名称", &mut form.name, "为这个连接起个名字", full);
        if form.kind == Kind::Serial {
            ui.horizontal(|ui| {
                field(ui, "串口设备", &mut form.device, "/dev/ttyUSB0 或 COM3", host_width);
                field(ui, "波特率", &mut form.baud, "115200", 110.0);
            });
            egui::Frame::new().fill(style::alpha(palette.tokens.accent, 0.08)).corner_radius(6).inner_margin(8).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(icon(icons::INFO, 16.0).color(palette.accent()));
                    ui.label(RichText::new("使用本地 Python 与 pyserial。连接后按 Ctrl+] 退出串口控制台。").size(11.0));
                });
            });
        } else {
            ui.horizontal(|ui| {
                field(ui, "主机地址", &mut form.host, "主机名或 IP 地址", host_width);
                field(ui, "SSH 端口", &mut form.port, "22", 110.0);
            });
            field(ui, "用户名", &mut form.username, "留空时使用系统默认用户", full);
            field(ui, "私钥路径（可选）", &mut form.key_path, "例如 ~/.ssh/id_ed25519", full);
            ui.label(RichText::new("也可以使用系统 SSH 配置和 SSH Agent。").size(10.0).color(palette.faint()));
            field(ui, "跳板机（可选）", &mut form.proxy_jump, "例如 root@192.168.10.222 或 ~/.ssh/config 中的别名", full);
            ui.label(RichText::new("对应 ProxyJump，多级跳板用逗号分隔。留空时沿用 ~/.ssh/config 中的设置。").size(10.0).color(palette.faint()));
        }
        if form.kind == Kind::Tunnel {
            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(host_width);
                    ui.label(RichText::new("转发类型").size(11.0).color(palette.muted()));
                    let label = |forward: Forward| match forward {
                        Forward::L => "本地转发（-L）",
                        Forward::R => "远程转发（-R）",
                        Forward::D => "动态 SOCKS（-D）",
                    };
                    egui::ComboBox::from_id_salt("forward").width(host_width).selected_text(label(form.forward)).show_ui(ui, |ui| {
                        for forward in [Forward::L, Forward::R, Forward::D] {
                            ui.selectable_value(&mut form.forward, forward, label(forward));
                        }
                    });
                });
                field(ui, "监听端口", &mut form.bind_port, "8080", 110.0);
            });
            if form.forward != Forward::D {
                ui.horizontal(|ui| {
                    field(ui, "目标主机", &mut form.target_host, "localhost", host_width);
                    field(ui, "目标端口", &mut form.target_port, "3000", 110.0);
                });
            }
        }
        ui.add_space(8.0);
        ui.checkbox(&mut form.persist, "保存到我的连接");
        if let Some(error) = &form.error {
            ui.colored_label(palette.danger(), error);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let mut chosen = None;
            if style::secondary_button(ui, "仅保存").clicked() {
                chosen = Some(false);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if style::primary_button(ui, &format!("{}  连接", icons::TERMINAL_WINDOW)).clicked() || ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    chosen = Some(true);
                }
                if style::ghost_button(ui, "取消").clicked() {
                    intents.push(Intent::CloseDialog);
                }
            });
            if let Some(connect) = chosen {
                match form.profile() {
                    Ok(profile) => intents.push(Intent::SaveProfile { profile, connect, persist: form.persist }),
                    Err(error) => form.error = Some(error),
                }
            }
        });
    })
}

fn delete(profile: &Profile, error: Option<&str>, ctx: &egui::Context, intents: &mut Vec<Intent>) -> bool {
    let palette = style::palette_of(ctx);
    let name = profile.display_name();
    modal(ctx, "delete", 400.0, icons::TRASH, "删除保存的连接？", &name, |ui| {
        ui.label(RichText::new("连接配置将从此设备移除，已经打开的会话不受影响。").size(12.0));
        if let Some(error) = error {
            ui.colored_label(palette.danger(), error);
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if style::danger_button(ui, "删除连接").clicked()
                && let Some(id) = &profile.id
            {
                intents.push(Intent::DeleteProfile(id.clone()));
            }
            if style::ghost_button(ui, "取消").clicked() {
                intents.push(Intent::CloseDialog);
            }
        });
    })
}

fn import(state: &mut ImportState, ctx: &egui::Context, intents: &mut Vec<Intent>) -> bool {
    let palette = style::palette_of(ctx);
    let subtitle = format!("在本机 SSH 配置中发现 {} 个主机", state.hosts.len());
    modal(ctx, "ssh-import", 460.0, icons::HARD_DRIVES, "导入 SSH 配置？", &subtitle, |ui| {
        ui.add(egui::Label::new(RichText::new(format!("所选主机会加入“已保存的连接”。连接时由系统 ssh 读取 {}，其中的跳板机、密钥等设置继续生效；不会读取或保存密码和私钥内容。", state.path.display())).size(11.5)).wrap());
        ui.add_space(8.0);
        if state.hosts.len() > 1 {
            let mut all = state.selected.len() == state.hosts.len();
            if ui.checkbox(&mut all, "全选").changed() {
                state.selected = if all { state.hosts.iter().map(|host| host.alias.clone()).collect() } else { BTreeSet::new() };
            }
        }
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for host in &state.hosts {
                let mut on = state.selected.contains(&host.alias);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").changed() {
                        if on {
                            state.selected.insert(host.alias.clone());
                        } else {
                            state.selected.remove(&host.alias);
                        }
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&host.alias).strong().color(palette.text()));
                        ui.label(RichText::new(host.summary()).size(10.5).color(palette.faint()));
                    });
                });
            }
        });
        if let Some(error) = &state.error {
            ui.colored_label(palette.danger(), error);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if state.startup && style::ghost_button(ui, "不再询问").clicked() {
                intents.push(Intent::NeverAskImport);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = state.selected.len();
                if ui.add_enabled(count > 0, egui::Button::new(RichText::new(format!("导入 {count} 个主机")).strong())).clicked() {
                    intents.push(Intent::ImportHosts { aliases: state.selected.iter().cloned().collect() });
                }
                if style::ghost_button(ui, "暂不导入").clicked() {
                    intents.push(Intent::CloseDialog);
                }
            });
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_settings_fit_without_changing_paths_or_arguments() {
        for width in [320.0, 560.0] {
            let ctx = egui::Context::default();
            let mut shell = LocalShell { program: "a very long program path with spaces/".repeat(12), args: vec!["literal argument with spaces; $(not executed)".repeat(12)] };
            let original = shell.clone();
            ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 800.0))), ..Default::default() }, |ui| {
                shell_settings(ui, &mut shell);
                assert!(ui.min_rect().right() <= width + 0.5, "shell form overflowed {width}px");
            }).drop_without_applying_deltas();
            assert_eq!(shell, original);
        }
    }
}
