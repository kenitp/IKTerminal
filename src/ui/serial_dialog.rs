//! Dialog that picks a COM port and baud rate.

use egui::RichText;

use super::theme;
use super::widgets::primary_button;
use crate::backend::serial;

pub struct SerialOpen {
    pub port: String,
    pub baud: u32,
}

pub enum SerialResult {
    Open(SerialOpen),
    Closed,
}

pub struct SerialDialog {
    ports: Vec<String>,
    port: String,
    baud: u32,
    error: Option<String>,
}

impl SerialDialog {
    pub fn new() -> Self {
        let ports = serial::list_ports();
        let port = ports.first().cloned().unwrap_or_default();
        Self {
            ports,
            port,
            baud: 115_200,
            error: None,
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<SerialResult> {
        let mut result = None;
        egui::Modal::new(egui::Id::new("serial")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("シリアル接続");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("ポート");
                egui::ComboBox::from_id_salt("serial-port")
                    .selected_text(if self.port.is_empty() {
                        "ポートがありません"
                    } else {
                        self.port.as_str()
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for port in &self.ports {
                            ui.selectable_value(&mut self.port, port.clone(), port);
                        }
                    });
                if ui.button("再読込").clicked() {
                    self.ports = serial::list_ports();
                    if !self.ports.iter().any(|p| p == &self.port) {
                        self.port = self.ports.first().cloned().unwrap_or_default();
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("ボーレート");
                egui::ComboBox::from_id_salt("serial-baud")
                    .selected_text(self.baud.to_string())
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for baud in serial::baud_rates() {
                            ui.selectable_value(&mut self.baud, *baud, baud.to_string());
                        }
                    });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("8 ビット、パリティなし、ストップビット 1、フロー制御なし")
                    .size(11.5)
                    .color(theme::TEXT_DIM),
            );
            if let Some(e) = &self.error {
                ui.label(RichText::new(e).color(theme::DANGER));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if primary_button(ui, "接続").clicked() {
                    if self.port.is_empty() {
                        self.error = Some("ポートを選択してください".to_owned());
                    } else {
                        result = Some(SerialResult::Open(SerialOpen {
                            port: self.port.clone(),
                            baud: self.baud,
                        }));
                    }
                }
                if ui.button("キャンセル").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    result = Some(SerialResult::Closed);
                }
            });
        });
        result
    }
}
