//! La ventana: ficha de la reunión, grabar / pausar / detener y el resultado.

use crate::audio::Recorder;
use crate::pipeline::{self, Event, Job};
use crate::settings::Settings;
use crate::transcribe::Progress;
use chrono::{DateTime, Local};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

enum State {
    /// Ficha de la reunión, antes de grabar.
    Setup,
    /// Grabando (o en pausa).
    Recording { recorder: Recorder, dir: PathBuf, started_at: DateTime<Local> },
    /// Transcribiendo en segundo plano.
    Transcribing { dir: PathBuf, rx: Receiver<Event>, status: String, percent: Option<u32>, seconds: f64 },
    /// Terminado: muestra dónde ha quedado todo.
    Done { dir: PathBuf, message: String, transcript: Option<PathBuf> },
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
                self.state = State::Recording { recorder, dir, started_at: Local::now() };
            }
            Err(err) => self.error = Some(format!("No se pudo empezar a grabar: {err:#}")),
        }
    }

    fn stop_recording(&mut self) {
        let state = std::mem::replace(&mut self.state, State::Setup);
        if let State::Recording { recorder, dir, started_at } = state {
            let seconds = recorder.recorded_seconds();
            if let Err(err) = recorder.stop() {
                self.state = State::Done {
                    dir,
                    message: format!("La grabación terminó con un error: {err:#}"),
                    transcript: None,
                };
                return;
            }
            let job = Job {
                dir: dir.clone(),
                title: self.meeting_name.trim().to_string(),
                started_at,
                my_name: self.settings.my_name.clone(),
                other_name: if self.other_name.trim().is_empty() { self.settings.other_name.clone() } else { self.other_name.trim().to_string() },
                settings: self.settings.clone(),
            };
            let rx = pipeline::run(job);
            self.state = State::Transcribing { dir, rx, status: "Preparando…".into(), percent: None, seconds };
        }
    }

    /// Recoge los avisos del hilo de transcripción.
    fn poll_transcription(&mut self) {
        let finished = if let State::Transcribing { rx, status, percent, .. } = &mut self.state {
            let mut finished = None;
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Progress(Progress::Status(text)) => {
                        *status = text;
                        *percent = None;
                    }
                    Event::Progress(Progress::Percent(p)) => *percent = Some(p),
                    Event::Finished(result) => finished = Some(result),
                }
            }
            finished
        } else {
            None
        };
        if let Some(result) = finished {
            let state = std::mem::replace(&mut self.state, State::Setup);
            if let State::Transcribing { dir, seconds, .. } = state {
                self.state = match result {
                    Ok(path) => State::Done {
                        dir,
                        message: format!("{} de reunión transcritos.", format_duration(seconds)),
                        transcript: Some(path),
                    },
                    Err(err) => State::Done {
                        dir,
                        message: format!("Las grabaciones están guardadas, pero la transcripción falló: {err:#}"),
                        transcript: None,
                    },
                };
            }
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

            self.poll_transcription();
            match &self.state {
                State::Setup => self.setup_ui(ui),
                State::Recording { .. } => self.recording_ui(ui, ctx),
                State::Transcribing { .. } => self.transcribing_ui(ui, ctx),
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

    fn transcribing_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (status, percent) = match &self.state {
            State::Transcribing { status, percent, .. } => (status.clone(), *percent),
            _ => unreachable!(),
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        ui.label(egui::RichText::new("Transcribiendo").size(18.0));
        ui.add_space(6.0);
        ui.label(status);
        ui.add_space(8.0);
        match percent {
            Some(p) => {
                ui.add(egui::ProgressBar::new(p as f32 / 100.0).show_percentage());
            }
            None => {
                ui.add(egui::ProgressBar::new(0.0).animate(true));
            }
        }
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Las grabaciones ya están guardadas; puedes cerrar el programa de la reunión.").weak());
    }

    fn done_ui(&mut self, ui: &mut egui::Ui) {
        let (dir, message, transcript) = match &self.state {
            State::Done { dir, message, transcript } => (dir.clone(), message.clone(), transcript.clone()),
            _ => unreachable!(),
        };
        ui.label(egui::RichText::new("Reunión terminada").size(18.0));
        ui.add_space(6.0);
        ui.label(message);
        ui.label(egui::RichText::new(dir.display().to_string()).weak());
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if let Some(path) = &transcript {
                if ui.button("Abrir la transcripción").clicked() {
                    let _ = std::process::Command::new("cmd").args(["/C", "start", "", &path.display().to_string()]).spawn();
                }
            }
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
                ui.label("Palabras habituales");
                let mut vocab = self.settings.vocabulary.join(", ");
                if ui.add(egui::TextEdit::singleline(&mut vocab).desired_width(300.0)).changed() {
                    self.settings.vocabulary = vocab.split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
                }
                ui.end_row();
            });
            ui.label(egui::RichText::new("Las palabras habituales (nombres, marcas) ayudan a Whisper a escribirlas bien. Separadas por comas.").weak());
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
