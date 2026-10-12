//! Address range for the LAN scan, offered per network interface.

use std::net::Ipv4Addr;

use egui::RichText;

use super::theme;
use super::widgets::primary_button;
use crate::backend::lan::Range;
use crate::backend::netif::Interface;

/// Ranges with more hosts than this get a warning.
const WIDE: u32 = 254;

pub enum ScanResult {
    Start(Range),
    Closed,
}

struct Group {
    label: String,
    /// Indices into `ScanDialog::presets`.
    presets: std::ops::Range<usize>,
}

pub struct ScanDialog {
    groups: Vec<Group>,
    /// Each preset and this machine's address on that interface.
    presets: Vec<(Range, Ipv4Addr)>,
    locals: Vec<Ipv4Addr>,
    /// Index into `presets`; `presets.len()` is the typed range.
    choice: usize,
    custom: String,
    error: Option<String>,
}

impl ScanDialog {
    /// The interface that holds `default_route` comes first. `last` is preselected; otherwise
    /// the first choice of the first interface.
    pub fn new(
        interfaces: &[Interface],
        default_route: Option<Ipv4Addr>,
        last: Option<Range>,
    ) -> Self {
        let mut ordered: Vec<&Interface> = interfaces.iter().collect();
        ordered.sort_by_key(|interface| Some(interface.addr) != default_route);
        let mut groups = Vec::new();
        let mut presets = Vec::new();
        for interface in ordered {
            let ranges = Range::for_interface(interface.addr, interface.prefix);
            if ranges.is_empty() {
                continue;
            }
            let start = presets.len();
            presets.extend(ranges.into_iter().map(|range| (range, interface.addr)));
            groups.push(Group {
                label: format!("{}  {}", interface.name, interface.addr),
                presets: start..presets.len(),
            });
        }
        let (choice, custom) = match last {
            Some(last) => match presets.iter().position(|&(range, _)| range == last) {
                Some(at) => (at, String::new()),
                None => (presets.len(), last.to_string()),
            },
            None => (0, String::new()),
        };
        Self {
            groups,
            presets,
            locals: interfaces.iter().map(|interface| interface.addr).collect(),
            choice,
            custom,
            error: None,
        }
    }

    fn selected(&self) -> Result<(Range, Option<Ipv4Addr>), String> {
        match self.presets.get(self.choice) {
            Some(&(range, local)) => Ok((range, Some(local))),
            None => {
                let range = Range::parse(&self.custom)?;
                let local = self.locals.iter().copied().find(|&a| range.contains(a));
                Ok((range, local))
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<ScanResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new(("lan-scan", ctx.viewport_id()))).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("LAN スキャン");
            if self.groups.is_empty() {
                ui.label(
                    RichText::new("プライベート IPv4 アドレスを持つ NIC がありません")
                        .color(theme::TEXT_DIM)
                        .size(12.5),
                );
            }
            ui.add_space(4.0);
            for group in &self.groups {
                ui.label(RichText::new(&group.label).color(theme::TEXT_DIM).size(12.5));
                ui.indent(&group.label, |ui| {
                    for at in group.presets.clone() {
                        let (range, local) = self.presets[at];
                        let label = format!("{range}  ({} 台)", range.count(Some(local)));
                        ui.radio_value(&mut self.choice, at, label);
                    }
                });
            }
            let custom_at = self.presets.len();
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.choice, custom_at, "指定");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.custom)
                        .hint_text("192.168.1.0/24")
                        .desired_width(200.0),
                );
                if edit.gained_focus() || edit.changed() {
                    self.choice = custom_at;
                }
            });
            if let Ok((range, local)) = self.selected()
                && range.count(local) > WIDE
            {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} 台を調べます。社内網ではポートスキャンとして通信を止められることがあります",
                        range.count(local)
                    ))
                    .size(11.5)
                    .color(theme::WARNING),
                );
            }
            if let Some(error) = &self.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "スキャン").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    match self.selected() {
                        Ok((range, _)) => result = Some(ScanResult::Start(range)),
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.button("キャンセル").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    result = Some(ScanResult::Closed);
                }
            });
        });
        result
    }
}
