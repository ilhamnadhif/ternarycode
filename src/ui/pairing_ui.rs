//! Settings → Phone: the companion server (on/off, its port, a tunnel address), the QR code a
//! phone scans to pair, and the paired phones (each can be revoked).

use eframe::egui::{self, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use tc_core::status::Status;

use super::settings::{any_hit, galley, hit, mono_field, raised, section, text};
use super::{icons, kit, style};
use crate::actions::Action;
use crate::app::App;
use crate::i18n::{t, tf};
use crate::qr::QrCode;

impl App {
    /// Settings → Phone.
    pub(super) fn phone_page(&mut self, ui: &mut Ui, search: &str) {
        let p = style::pal(ui);
        let dark = ui.visuals().dark_mode;
        let running = self.companion.is_some();

        // The server, remote Cloudflare relay, port, and optional custom tunnel override.
        let port_draft_id = egui::Id::new("companion-port-draft");
        let port_s = ui.data(|d| d.get_temp::<String>(port_draft_id)).unwrap_or_else(|| self.settings.companion.port.to_string());
        let port_bad = port_s.trim().parse::<u16>().ok().filter(|&v| v >= 1024).is_none();
        let tunnel = self.settings.companion.tunnel_url.trim().to_string();
        let tunnel_bad = !tunnel.is_empty() && !tunnel.starts_with("http://") && !tunnel.starts_with("https://");
        let relay_status = self
            .companion
            .as_ref()
            .map(crate::companion::CompanionHandle::relay_status)
            .unwrap_or_default();
        let labels = [
            (t("Companion server"), t("Serves a small web app on your local network. Each phone gets its own key.")),
            (t("Remote access"), t("Reach this computer from anywhere via Cloudflare Relay (works outside Wi-Fi).")),
            (t("Port"), ""),
            (t("Tunnel address"), t("Optional override: Tailscale, ngrok or custom URL.")),
        ];
        let mut toggle = false;
        let mut toggle_relay = false;
        section(ui, search, None, None, &labels, |c| {
            c.row(labels[0].0, labels[0].1, |ui| {
                let (st, label) = if running { (Status::Done, t("Online")) } else { (Status::Idle, t("Off")) };
                let w = kit::state_pill_width(ui.painter(), st, label);
                let (r, _) = ui.allocate_exact_size(vec2(w, kit::PILL_H), Sense::hover());
                kit::paint_state_pill(ui.painter(), r.left_center(), st, label, &p, dark);
                let mut enabled = self.settings.companion.enabled;
                let s = kit::switch(ui, &mut enabled);
                #[cfg(debug_assertions)]
                crate::devtest::record("companion-switch", s.rect);
                toggle = s.changed();
            });
            let relay_err = match &relay_status {
                crate::companion::RelayStatus::Error(msg) if running => Some(msg.as_str()),
                _ => None,
            };
            let mut relay_row = |ui: &mut Ui, app: &App| {
                if running {
                    let (st, label) = match &relay_status {
                        crate::companion::RelayStatus::Connected => (Status::Done, t("Connected")),
                        crate::companion::RelayStatus::Connecting => (Status::Running, t("Connecting")),
                        crate::companion::RelayStatus::Error(_) => (Status::Failed(1), t("Offline")),
                        crate::companion::RelayStatus::Off => (Status::Idle, t("Off")),
                    };
                    let w = kit::state_pill_width(ui.painter(), st, label);
                    let (r, resp) = ui.allocate_exact_size(vec2(w, kit::PILL_H), Sense::hover());
                    kit::paint_state_pill(ui.painter(), r.left_center(), st, label, &p, dark);
                    if let Some(err) = relay_err {
                        resp.on_hover_text(err);
                    }
                }
                let mut rel = running && app.settings.companion.relay_active();
                let s = kit::switch(ui, &mut rel).on_disabled_hover_text(t("Turn on Companion server first."));
                #[cfg(debug_assertions)]
                crate::devtest::record("companion-relay-switch", s.rect);
                toggle_relay = running && s.changed();
            };
            if !running {
                c.row_disabled(
                    labels[1].0,
                    t("Turn on Companion server to reach this computer via Cloudflare Relay."),
                    |ui| relay_row(ui, self),
                );
            } else if let Some(err) = relay_err {
                c.row_note(labels[1].0, err, p.err, |ui| relay_row(ui, self));
            } else {
                c.row(labels[1].0, labels[1].1, |ui| relay_row(ui, self));
            }
            let port_row = |ui: &mut Ui, app: &mut App| {
                let port_id = egui::Id::new("companion-port");
                let mut text = ui.data(|d| d.get_temp::<String>(port_draft_id)).unwrap_or_else(|| app.settings.companion.port.to_string());
                let r = mono_field(ui, port_id, &mut text, "7890", 90.0, false);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let valid = text.trim().parse::<u16>().ok().filter(|&v| v >= 1024);
                if r.changed() {
                    ui.data_mut(|d| d.insert_temp(port_draft_id, text.clone()));
                }
                // The port is taken when the field is left (or Enter): a bad one goes back.
                if r.lost_focus() || enter {
                    if let Some(port) = valid
                        && port != app.settings.companion.port
                    {
                        app.settings.companion.port = port;
                        app.settings_changed();
                        app.sync_companion_server();
                    }
                    ui.data_mut(|d| d.remove::<String>(port_draft_id));
                } else if !r.has_focus() {
                    ui.data_mut(|d| d.remove::<String>(port_draft_id));
                }
            };
            if port_bad {
                c.row_note(labels[2].0, t("From 1024 to 65535."), p.err, |ui| port_row(ui, self));
            } else {
                c.row(labels[2].0, "", |ui| port_row(ui, self));
            }
            let note = if tunnel_bad { t("It starts with https:// or http://.") } else { labels[3].1 };
            let tunnel_row = |ui: &mut Ui, app: &mut App| {
                let mut url = app.settings.companion.tunnel_url.clone();
                if kit::field(ui, egui::Id::new("companion-tunnel"), &mut url, Some(icons::GLOBE_SIMPLE), "https://…", 240.0, false).changed() {
                    app.settings.companion.tunnel_url = url;
                    app.settings_changed();
                }
            };
            if tunnel_bad {
                c.row_note(labels[3].0, note, p.err, |ui| tunnel_row(ui, self));
            } else {
                c.row(labels[3].0, note, |ui| tunnel_row(ui, self));
            }
        });
        if toggle {
            self.actions.push(Action::ToggleCompanionServer);
        }
        if toggle_relay {
            self.actions.push(Action::ToggleCompanionRelay);
        }

        // The code to scan (while the server runs).
        let qr_labels = [(t("Scan with your phone's camera"), t("Every phone gets its own key, and you can revoke it any time."))];
        if any_hit(search, &qr_labels) {
            match self.companion.as_ref().map(|c| {
                let token = c.pair_token();
                let valid_tunnel = if tunnel_bad { "" } else { tunnel.as_str() };
                let has_remote = !valid_tunnel.is_empty() || c.relay_endpoint().is_some();
                (
                    c.pairing_url(valid_tunnel),
                    token.clone(),
                    c.active_endpoint(valid_tunnel),
                    format!("http://127.0.0.1:{}/?pair={token}", c.port),
                    c.local_ip == "127.0.0.1" && !has_remote,
                )
            }) {
                Some((pair_url, code, endpoint, preview, loopback)) => {
                    raised(ui, 16, |ui| {
                        let card_inner_w = ui.available_width();
                        let col_w = (card_inner_w - 176.0 - 24.0).max(160.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 24.0;
                            let (qr, _) = ui.allocate_exact_size(vec2(176.0, 176.0), Sense::hover());
                            #[cfg(debug_assertions)]
                            crate::devtest::record("pairing-qr", qr);
                            paint_qr_card(ui, qr, &pair_url);
                            // The text beside the code, centred on it (its height from the last frame).
                            let text_h_id = egui::Id::new("pairing-text-h");
                            let text_h = ui.data(|d| d.get_temp::<f32>(text_h_id)).unwrap_or(110.0);
                            let block = ui.vertical(|ui| {
                                ui.set_width(col_w);
                                ui.set_max_width(col_w);
                                ui.add_space(((176.0 - text_h) / 2.0).max(0.0));
                                ui.spacing_mut().item_spacing.y = 10.0;
                                text(ui, qr_labels[0].0, style::semibold(style::HEADING), p.text, 20.0, true);
                                text(ui, qr_labels[0].1, FontId::proportional(style::SMALL), p.text_muted, 16.0, true);
                                if loopback {
                                    text(ui, t("No Wi-Fi address found: join a network, or add a tunnel address above."), FontId::proportional(style::SMALL), p.warn, 16.0, true);
                                }
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                                    let g = galley(ui, &code, style::mono_semibold(style::DATA), p.text, None, 14.0);
                                    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 24.0, 30.0), Sense::hover());
                                    ui.painter().rect_filled(r, CornerRadius::same(7), p.inset);
                                    ui.painter().galley(r.center() - g.size() / 2.0, g, p.text);
                                    if kit::button_ex(ui, kit::Kind::Secondary, Some(icons::COPY), t("Copy link"), true).clicked() {
                                        ui.ctx().copy_text(pair_url.clone());
                                        self.toast_ok(t("Pairing link copied.").into());
                                    }
                                    if kit::button_ex(ui, kit::Kind::Ghost, Some(icons::ARROWS_CLOCKWISE), t("New code"), true).clicked() {
                                        self.actions.push(Action::RegeneratePairingToken);
                                    }
                                });
                                // The address: truncated cleanly inside the card width; full URL on hover, click opens browser.
                                let mut job = egui::text::LayoutJob::default();
                                let mut fmt = egui::TextFormat::simple(style::mono(style::DATA), p.text_faint);
                                fmt.line_height = Some(14.0);
                                job.append(&endpoint, 0.0, fmt);
                                job.wrap = egui::text::TextWrapping::truncate_at_width(col_w);
                                let g = ui.painter().layout_job(job);
                                let (r, resp) = ui.allocate_exact_size(vec2(g.size().x.min(col_w), g.size().y), Sense::click());
                                #[cfg(debug_assertions)]
                                crate::devtest::record("pairing-endpoint", r);
                                let c = if resp.hovered() { p.accent_text } else { p.text_faint };
                                ui.painter().galley(r.min, g, c);
                                let tip = format!("{}\n{endpoint}", t("Open the phone's app here"));
                                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                                    crate::platform::open_url(&preview);
                                }
                                r.bottom()
                            });
                            let h = block.inner - block.response.rect.top() - ((176.0 - text_h) / 2.0).max(0.0);
                            if (h - text_h).abs() > 0.5 {
                                ui.data_mut(|d| d.insert_temp(text_h_id, h));
                                ui.ctx().request_repaint();
                            }
                        });
                    });
                }
                None if search.trim().is_empty() => {
                    raised(ui, 16, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 14.0;
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icons::DEVICE_MOBILE, style::icon_font(18.0), p.text_muted);
                            text(ui, t("Turn on the companion server to pair a phone: a QR code shows here."), FontId::proportional(style::SMALL), p.text_muted, 16.0, true);
                        });
                    });
                }
                None => {}
            }
        }

        // The phones paired, each with Revoke (asked first, in its row).
        let devices = self.settings.companion.devices.clone();
        let mut labels: Vec<(&str, &str)> = devices.iter().map(|d| (d.name.as_str(), d.ip.as_str())).collect();
        labels.push((t("Paired phones"), ""));
        let confirm_id = egui::Id::new("phone-revoke-confirm");
        let mut confirm: Option<String> = ui.data(|d| d.get_temp(confirm_id));
        let mut revoke = None;
        section(ui, search, Some(t("Paired phones")), None, &labels, |c| {
            if devices.is_empty() {
                c.block(&[], egui::Margin::symmetric(16, 12), |ui| {
                    text(ui, t("No phones yet. Scan the code above with a phone's camera to pair it."), FontId::proportional(style::SMALL), p.text_faint, 16.0, true);
                });
                return;
            }
            for dev in &devices {
                if !hit(c.search(), &dev.name, &dev.ip) && !hit(c.search(), t("Paired phones"), "") {
                    continue;
                }
                c.block(&[], egui::Margin::symmetric(16, 10), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 34.0), Sense::hover());
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icons::DEVICE_MOBILE, style::icon_font(18.0), p.text_muted);
                        let asking = confirm.as_deref() == Some(dev.id.as_str());
                        let right_w = if asking {
                            kit::button_width(ui, None, t("Cancel"), None, true) + 8.0 + kit::button_width(ui, None, t("Revoke"), None, true)
                        } else {
                            kit::button_width(ui, None, t("Revoke"), None, true)
                        };
                        let room = (ui.available_width() - right_w - 12.0).max(80.0);
                        ui.vertical(|ui| {
                            ui.set_width(room);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let name = if asking { tf("Revoke {}? It must pair again to connect.", &[&dev.name]) } else { dev.name.clone() };
                            text(ui, &name, FontId::proportional(style::BODY), p.text, 18.0, true);
                            let seen = format!("{} · {}", dev.ip, seen_ago(dev.last_seen));
                            text(ui, &seen, style::mono(style::DATA), p.text_faint, 14.0, false);
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let r = kit::button_ex(ui, kit::Kind::Danger, None, t("Revoke"), true);
                            #[cfg(debug_assertions)]
                            crate::devtest::record(format!("revoke:{}", dev.id), r.rect);
                            if r.clicked() {
                                if asking {
                                    revoke = Some(dev.id.clone());
                                    confirm = None;
                                } else {
                                    confirm = Some(dev.id.clone());
                                }
                            }
                            if asking && kit::button_ex(ui, kit::Kind::Ghost, None, t("Cancel"), true).clicked() {
                                confirm = None;
                            }
                        });
                    });
                });
            }
        });
        ui.data_mut(|d| match &confirm {
            Some(c) => {
                d.insert_temp(confirm_id, c.clone());
            }
            None => d.remove::<String>(confirm_id),
        });
        if let Some(id) = revoke {
            self.actions.push(Action::RevokeDevice(id));
        }
    }
}

/// "seen 2 min ago" for a Unix time.
fn seen_ago(at: i64) -> String {
    if at <= 0 {
        return t("not seen yet").to_string();
    }
    let s = (crate::profiles::now_unix_secs() - at).max(0);
    if s < 60 {
        t("seen just now").to_string()
    } else if s < 3600 {
        tf("seen {} min ago", &[&(s / 60).to_string()])
    } else if s < 86_400 {
        tf("seen {} h ago", &[&(s / 3600).to_string()])
    } else {
        tf("seen {} d ago", &[&(s / 86_400).to_string()])
    }
}

/// The QR code on white (radius 10), its quiet zone around it; cached per address.
fn paint_qr_card(ui: &mut Ui, rect: Rect, payload: &str) {
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(10), Color32::WHITE, Stroke::NONE, StrokeKind::Inside);
    let cache_id = egui::Id::new("qr-cache");
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
    let quiet = 2usize;
    let total = qr.size + quiet * 2;
    let inner = rect.shrink(6.0);
    let cell = (inner.width().min(inner.height()) / total as f32).floor().max(1.0);
    let grid_px = cell * total as f32;
    let ox = (inner.center().x - grid_px / 2.0).round();
    let oy = (inner.center().y - grid_px / 2.0).round();
    let ink = Color32::from_rgb(0x0b, 0x0c, 0x0e);
    let qp = painter.with_clip_rect(inner.intersect(painter.clip_rect()));
    for y in 0..qr.size {
        for x in 0..qr.size {
            if qr.get(x, y) {
                let r = Rect::from_min_size(pos2(ox + (x + quiet) as f32 * cell, oy + (y + quiet) as f32 * cell), vec2(cell, cell));
                qp.rect_filled(r, CornerRadius::ZERO, ink);
            }
        }
    }
}
