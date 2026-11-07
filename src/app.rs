//! La ventana: ficha de la reunión, grabar / pausar / detener y el resultado.

use crate::audio::Recorder;
use crate::settings::Settings;
use chrono::Local;
use eframe::egui;
use std::path::PathBuf;

enum State {
    /// Ficha de la reunión, antes de grabar.
    Setup,
    /// Grabando (o en pausa).
    Recording { recorder: Recorder, dir: PathBuf },
    /// Terminado: muestra dónde ha quedado todo.
    Done { dir: PathBuf, message: String },
}

pub struct MinutaApp {
    settings: Settings,
    meeting_name: String,
    other_name: String,
    state: State,
    error: Option<String>,
    show_settings: bool,
}

impl MinutaApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_pixels_per_point(1.25);
        let settings = Settings::load();
        Self {
            meeting_name: default_meeting_name(),
            other_name: settings.other_name.clone(),
            settings,
            state: State::Setup,
            error: None,
            show_settings: false,
        }
    }

    fn start_recording(&mut self) {
        let name = sanitize(self.meeting_name.trim());
        if name.is_empty() {
            self.error = Some("Ponle un nombre a la reunión.".into());
            return;
        }
        let dir = self.settings.base_dir.join(&name);
        if dir.exists() {
            self.error = Some(format!("Ya existe una carpeta «{name}». Cambia el nombre."));
            return;
        }
        let mic = dir.join(&self.settings.my_recording_file);
        let speaker = dir.join(&self.settings.other_recording_file);
        match Recorder::start(&mic, &speaker) {
            Ok(recorder) => {
                self.error = None;
                self.state = State::Recording { recorder, dir };
            }
            Err(err) => self.error = Some(format!("No se pudo empezar a grabar: {err:#}")),
        }
    }

    fn stop_recording(&mut self) {
        let state = std::mem::replace(&mut self.state, State::Setup);
        if let State::Recording { recorder, dir } = state {
            let seconds = recorder.recorded_seconds();
            let message = match recorder.stop() {
                Ok(()) => format!("Grabados {} de audio en dos pistas.", format_duration(seconds)),
                Err(err) => format!("La grabación terminó con un error: {err:#}"),
            };
            self.state = State::Done { dir, message };
        }
    }
}

impl eframe::App for MinutaApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        egui::CentralPanel::default().show(root, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading("minuta");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Ajustes").clicked() {
                        self.show_settings = !self.show_settings;
                    }
                });
            });
            ui.separator();
            ui.add_space(8.0);

            match &self.state {
                State::Setup => self.setup_ui(ui),
                State::Recording { .. } => self.recording_ui(ui, ctx),
                State::Done { .. } => self.done_ui(ui),
            }

            if let Some(err) = &self.error {
                ui.add_space(12.0);
                ui.colored_label(egui::Color32::from_rgb(200, 80, 80), err);
            }
        });

        if self.show_settings {
            self.settings_window(ctx);
        }
    }
}

impl MinutaApp {
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("ficha").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("Reunión");
            ui.add(egui::TextEdit::singleline(&mut self.meeting_name).desired_width(320.0));
            ui.end_row();
            ui.label("Con quién");
            ui.add(egui::TextEdit::singleline(&mut self.other_name).desired_width(320.0));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!(
                "Se guardará en {}",
                self.settings.base_dir.join(sanitize(self.meeting_name.trim())).display()
            ))
            .weak(),
        );
        ui.add_space(16.0);
        let button = egui::Button::new(egui::RichText::new("●  Grabar").size(18.0)).min_size([160.0, 40.0].into());
        if ui.add(button).clicked() {
            self.start_recording();
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Micrófono: tu voz · Altavoz: la otra persona. Mejor con cascos.").weak());
    }

    fn recording_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (seconds, paused, thread_error) = match &self.state {
            State::Recording { recorder, .. } => (recorder.recorded_seconds(), recorder.is_paused(), recorder.error()),
            _ => unreachable!(),
        };
        if let Some(err) = thread_error {
            self.error = Some(err);
            self.stop_recording();
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));

        let status = if paused { "En pausa" } else { "Grabando" };
        let color = if paused { egui::Color32::GRAY } else { egui::Color32::from_rgb(220, 60, 60) };
        ui.horizontal(|ui| {
            ui.colored_label(color, egui::RichText::new("●").size(22.0));
            ui.label(egui::RichText::new(status).size(18.0));
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format_duration(seconds)).size(40.0).monospace());
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            let pause_label = if paused { "▶  Reanudar" } else { "❚❚  Pausar" };
            if ui.add(egui::Button::new(pause_label).min_size([140.0, 36.0].into())).clicked() {
                if let State::Recording { recorder, .. } = &self.state {
                    recorder.set_paused(!paused);
                }
            }
            if ui.add(egui::Button::new("■  Detener").min_size([140.0, 36.0].into())).clicked() {
                self.stop_recording();
            }
        });
    }

    fn done_ui(&mut self, ui: &mut egui::Ui) {
        let (dir, message) = match &self.state {
            State::Done { dir, message } => (dir.clone(), message.clone()),
            _ => unreachable!(),
        };
        ui.label(egui::RichText::new("Reunión terminada").size(18.0));
        ui.add_space(6.0);
        ui.label(message);
        ui.label(egui::RichText::new(dir.display().to_string()).weak());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Abrir la carpeta").clicked() {
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            if ui.button("Nueva reunión").clicked() {
                self.meeting_name = default_meeting_name();
                self.error = None;
                self.state = State::Setup;
            }
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut save = false;
        egui::Window::new("Ajustes").open(&mut open).resizable(false).show(ctx, |ui| {
            egui::Grid::new("ajustes").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Carpeta base");
                ui.horizontal(|ui| {
                    let mut text = self.settings.base_dir.display().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0)).changed() {
                        self.settings.base_dir = PathBuf::from(text);
                    }
                    if ui.button("…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().set_directory(&self.settings.base_dir).pick_folder() {
                            self.settings.base_dir = dir;
                        }
                    }
                });
                ui.end_row();
                ui.label("Mi nombre");
                ui.text_edit_singleline(&mut self.settings.my_name);
                ui.end_row();
                ui.label("Otra persona (por defecto)");
                ui.text_edit_singleline(&mut self.settings.other_name);
                ui.end_row();
                ui.label("Archivo de mi grabación");
                ui.text_edit_singleline(&mut self.settings.my_recording_file);
                ui.end_row();
                ui.label("Archivo de la otra grabación");
                ui.text_edit_singleline(&mut self.settings.other_recording_file);
                ui.end_row();
                ui.label("Archivo de la transcripción");
                ui.text_edit_singleline(&mut self.settings.transcript_file);
                ui.end_row();
                ui.label("Idioma");
                ui.text_edit_singleline(&mut self.settings.language);
                ui.end_row();
                ui.label("Modelo de Whisper");
                ui.text_edit_singleline(&mut self.settings.model);
                ui.end_row();
            });
            ui.add_space(8.0);
            if ui.button("Guardar").clicked() {
                save = true;
            }
        });
        if save {
            match self.settings.save() {
                Ok(()) => {
                    self.other_name = self.settings.other_name.clone();
                    self.show_settings = false;
                }
                Err(err) => self.error = Some(format!("No se pudieron guardar los ajustes: {err:#}")),
            }
        }
        if !open {
            self.show_settings = false;
        }
    }
}

/// `AAMMDD_reunion`, como los archivos de `cliente/reuniones/`.
fn default_meeting_name() -> String {
    Local::now().format("%y%m%d_reunion").to_string()
}

/// Quita lo que no puede ir en un nombre de carpeta de Windows.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '-' } else { c })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

pub fn format_duration(seconds: f64) -> String {
    let total = seconds.floor() as u64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
}
