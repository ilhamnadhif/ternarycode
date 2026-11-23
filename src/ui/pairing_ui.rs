//! Phone Pairing & Mobile Companion management view (`View::Pairing`):
//! - Toggle local HTTP/SSE Mobile Companion server
//! - Render crisp native QR code + pairing token + optional tunnel URL
//! - Manage paired smartphones with instant revoke

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::qr::QrCode;

impl App {
    /// Dedicated Phone Pairing & Mobile Companion management page (`View::Pairing`).
    pub fn pairing_view(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let full = ui.max_rect();
        let body = self.top_bar(
            ui,
            full,
            |app, ui| {
                app.nav_back_button(ui);
                ui.label(RichText::new(t("Phone Pairing & Mobile Companion")).font(style::semibold(14.0)).color(p.text));
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(t("Monitor agents, approve tools, and trigger profile failover remotely from your phone"))
                            .size(12.0)
                            .color(p.text_faint),
                    )
                    .truncate(),
                );
            },
            None,
        );
        let area = super::focus::sheets_area(body, self.settings.sidebar_hidden);
        super::focus::paint_sheet(ui.painter(), area, &p, super::focus::SheetLook::Plain);

        let inner = area.shrink(1.0);
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            egui::ScrollArea::vertical()
                .id_salt("pairing-view-scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(24, 20))
                        .show(ui, |ui| {
                            ui.set_width((inner.width() - 48.0).max(320.0));
                            self.pairing_content(ui);
                        });
                });
        });
    }

    /// Reusable Phone Pairing panel body (rendered in `View::Pairing` and accessible from Settings).
    pub fn pairing_content(&mut self, ui: &mut Ui) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let running = self.companion.is_some();

        // 1. Companion server controls & network / tunnel configuration.
        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::Sides::new().spacing(14.0).shrink_left().wrap().show(
                    ui,
                    |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                                ui.label(RichText::new(t("Mobile Companion Server")).font(style::semibold(13.5)).color(p.text));
                                kit::status_badge(
                                    ui,
                                    if running { t("Online") } else { t("Stopped") },
                                    if running { p.ok } else { p.idle },
                                    false,
                                );
                            });
                            ui.add(
                                egui::Label::new(
                                    RichText::new(t("Serves a responsive mobile web companion over your local Wi-Fi or secure tunnel."))
                                        .size(12.0)
                                        .color(p.text_muted),
                                )
                                .wrap(),
                            );
                        });
                    },
                    |ui| {
                        let mut enabled = self.settings.companion.enabled;
                        kit::switch(ui, &mut enabled);
                        if enabled != self.settings.companion.enabled {
                            self.actions.push(Action::ToggleCompanionServer);
                        }
                    },
                );

                ui.add_space(12.0);
                let (sep, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(sep.x_range(), sep.center().y, Stroke::new(1.0, p.border));
                ui.add_space(12.0);

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(14.0, 8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Local Port")).size(11.5).color(p.text_muted));
                        let port_id = ui.id().with("companion-port");
                        let draft_id = ui.id().with("companion-port-draft");
                        let mut port_s = ui
                            .data(|d| d.get_temp::<String>(draft_id))
                            .unwrap_or_else(|| self.settings.companion.port.to_string());
                        let r = kit::field(ui, port_id, &mut port_s, None, "7890", 96.0, false);
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let valid = port_s.trim().parse::<u16>().ok().filter(|&v| v >= 1024);
                        if r.changed() {
                            ui.data_mut(|d| d.insert_temp(draft_id, port_s.clone()));
                        }
                        if r.lost_focus() || enter {
                            if let Some(port) = valid {
                                if port != self.settings.companion.port {
                                    self.settings.companion.port = port;
                                    self.settings_changed();
                                    self.sync_companion_server();
                                }
                                ui.data_mut(|d| d.insert_temp(draft_id, port.to_string()));
                            } else {
                                ui.data_mut(|d| d.insert_temp(draft_id, self.settings.companion.port.to_string()));
                            }
                        } else if !r.has_focus() {
                            ui.data_mut(|d| d.insert_temp(draft_id, self.settings.companion.port.to_string()));
                        }
                        if valid.is_none() && r.has_focus() {
                            ui.label(RichText::new("1024–65535").size(10.5).color(p.err));
                        }
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t("Secure Tunnel URL (optional — Cloudflare / Tailscale / ngrok)")).size(11.5).color(p.text_muted));
                        let mut tunnel = self.settings.companion.tunnel_url.clone();
                        let tunnel_w = (ui.available_width()).clamp(200.0, 360.0);
                        let r = kit::field(ui, ui.id().with("companion-tunnel"), &mut tunnel, Some(icons::GLOBE), "https://ade.example.com", tunnel_w, false);
                        if r.changed() {
                            self.settings.companion.tunnel_url = tunnel;
                            self.settings_changed();
                        }
                        let trimmed = self.settings.companion.tunnel_url.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                            ui.label(RichText::new(t("URL must start with https:// or http://")).size(10.5).color(p.err));
                        }
                    });
                });
            });

        ui.add_space(16.0);

        // 2. QR Code + Pairing Token card (when server is running).
        if let Some((pair_url, pair_code, local_addr, preview_url, loopback_only)) = self.companion.as_ref().map(|c| {
            let token = c.pair_token();
            let tunnel = self.settings.companion.tunnel_url.trim();
            let valid_tunnel = if tunnel.starts_with("http://") || tunnel.starts_with("https://") { tunnel } else { "" };
            (
                c.pairing_url(valid_tunnel),
                token.clone(),
                format!("http://{}:{}", c.local_ip, c.port),
                format!("http://127.0.0.1:{}/?pair={token}", c.port),
                c.local_ip == "127.0.0.1" && valid_tunnel.is_empty(),
            )
        }) {
            egui::Frame::NONE
                .fill(p.surface)
                .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
                .corner_radius(CornerRadius::same(9))
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    let card_w = ui.available_width();
                    ui.set_width(card_w);
                    let draw_qr = |ui: &mut Ui| {
                        // Crisp native QR code inside a high-contrast quiet zone (cached per URL).
                        let qr_box_size = 196.0;
                        let (qr_rect, _) = ui.allocate_exact_size(vec2(qr_box_size, qr_box_size), Sense::hover());
                        #[cfg(debug_assertions)]
                        crate::devtest::record("pairing-qr", qr_rect);
                        paint_qr_card(ui, qr_rect, &pair_url, &p);
                    };
                    let mut draw_details = |ui: &mut Ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            ui.label(RichText::new(t("Scan with your phone camera")).font(style::semibold(14.0)).color(p.text));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(t("Point your iPhone or Android camera at this QR code to pair securely. Every device receives its own revocable token."))
                                        .size(12.0)
                                        .color(p.text_muted),
                                )
                                .wrap(),
                            );
                            if loopback_only {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(t("No LAN IP detected — connect to Wi-Fi or configure a tunnel URL above."))
                                            .size(11.5)
                                            .color(p.warn),
                                    )
                                    .wrap(),
                                );
                            }
                            ui.add_space(4.0);
                            ui.label(RichText::new(t("Pairing Code")).size(11.0).color(p.text_faint));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                                egui::Frame::NONE
                                    .fill(p.inset)
                                    .stroke(Stroke::new(1.0, p.border_strong))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(egui::Margin::symmetric(10, 6))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(&pair_code).font(style::mono_semibold(14.0)).color(p.text));
                                    });
                                if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::COPY), t("Copy Link"), false).clicked() {
                                    ui.ctx().copy_text(pair_url.clone());
                                    self.toast_ok(t("Pairing link copied.").into());
                                }
                                if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("Regenerate"), false).clicked() {
                                    self.actions.push(Action::RegeneratePairingToken);
                                }
                            });

                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(tf("Local network endpoint: {}", &[&local_addr]))
                                        .font(style::mono(11.0))
                                        .color(p.text_faint),
                                )
                                .wrap(),
                            );
                            ui.horizontal(|ui| {
                                if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::ARROW_SQUARE_OUT), t("Preview Mobile UI"), true).clicked() {
                                    crate::platform::open_url(&preview_url);
                                }
                            });
                        });
                    };
                    if card_w < 580.0 {
                        draw_qr(ui);
                        ui.add_space(14.0);
                        draw_details(ui);
                    } else {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = 22.0;
                            draw_qr(ui);
                            draw_details(ui);
                        });
                    }
                });
            ui.add_space(16.0);
        }

        // 3. Connected Devices list with instant Revoke.
        let devices = self.settings.companion.devices.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("Paired Devices")).font(style::semibold(13.0)).color(p.text));
            ui.label(
                RichText::new(devices.len().to_string())
                    .font(style::mono(11.0))
                    .color(p.text_faint),
            );
        });
        ui.add_space(6.0);

        egui::Frame::NONE
            .fill(p.surface)
            .stroke(Stroke::new(1.0, if dark { p.border } else { p.border_strong }))
            .corner_radius(CornerRadius::same(9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if devices.is_empty() {
                    egui::Frame::NONE.inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                        ui.label(
                            RichText::new(t("No smartphones paired yet. Enable the server above and scan the QR code."))
                                .size(12.0)
                                .color(p.text_faint),
                        );
                    });
                } else {
                    for (i, dev) in devices.iter().enumerate() {
                        if i > 0 {
                            let y = ui.cursor().top();
                            ui.painter().hline(ui.max_rect().x_range(), y + 0.5, Stroke::new(1.0, p.border));
                        }
                        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            egui::Sides::new().spacing(12.0).shrink_left().wrap().show(
                                ui,
                                |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(icons::DEVICE_MOBILE).font(style::icon_font(16.0)).color(p.text_muted));
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 2.0;
                                            ui.label(RichText::new(&dev.name).font(style::medium(12.5)).color(p.text));
                                            let seen = chrono::DateTime::from_timestamp(dev.last_seen, 0)
                                                .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
                                                .unwrap_or_default();
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!("{} · {} {seen}", dev.ip, t("Last seen")))
                                                        .font(style::mono(10.5))
                                                        .color(p.text_faint),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                },
                                |ui| {
                                    if kit::button_ex(ui, kit::Kind::Danger, Some(icons::SIGN_OUT), t("Revoke"), true).clicked() {
                                        self.actions.push(Action::RevokeDevice(dev.id.clone()));
                                    }
                                },
                            );
                        });
                    }
                }
            });
    }
}

/// Paints a standards-compliant crisp QR code inside a high-contrast white quiet zone.
fn paint_qr_card(ui: &mut Ui, rect: Rect, payload: &str, p: &style::Palette) {
    let painter = ui.painter();
    let dark = ui.visuals().dark_mode;
    let border = if dark { Color32::from_gray(90) } else { p.border_heavy };
    painter.rect(rect, CornerRadius::same(10), Color32::WHITE, Stroke::new(1.0, border), StrokeKind::Inside);
    let cache_id = ui.id().with("qr-cache");
    let qr = match ui.data(|d| d.get_temp::<(String, Option<std::sync::Arc<QrCode>>)>(cache_id)) {
        Some((cached_url, cached)) if cached_url == payload => cached,
        _ => {
            let encoded = QrCode::encode(payload.as_bytes()).map(std::sync::Arc::new);
            ui.data_mut(|d| d.insert_temp(cache_id, (payload.to_string(), encoded.clone())));
            encoded
        }
    };
    let Some(qr) = qr else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, t("URL too long for QR"), style::medium(11.5), Color32::BLACK);
        return;
    };
    let quiet = 4usize;
    let total = qr.size + quiet * 2;
    let pad = 8.0;
    let inner = rect.shrink(pad);
    let cell = (inner.width().min(inner.height()) / total as f32).floor().max(2.0);
    let grid_px = cell * total as f32;
    let ox = (inner.center().x - grid_px / 2.0).round();
    let oy = (inner.center().y - grid_px / 2.0).round();
    let ink = Color32::from_rgb(0x0b, 0x0c, 0x0e);
    for y in 0..qr.size {
        for x in 0..qr.size {
            if qr.get(x, y) {
                let r = Rect::from_min_size(
                    pos2(ox + (x + quiet) as f32 * cell, oy + (y + quiet) as f32 * cell),
                    vec2(cell, cell),
                );
                painter.rect_filled(r, CornerRadius::ZERO, ink);
            }
        }
    }
}
