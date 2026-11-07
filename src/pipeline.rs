//! Lo que pasa al pulsar Detener: cargar el modelo (descargándolo si hace
//! falta), transcribir las dos pistas y escribir el Markdown. Corre en un hilo
//! aparte y va informando a la ventana por un canal.

use crate::merge::{Speaker, to_markdown};
use crate::model::ensure_model;
use crate::settings::Settings;
use crate::transcribe::{Model, Progress};
use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

pub struct Job {
    pub dir: PathBuf,
    pub title: String,
    pub started_at: DateTime<Local>,
    pub my_name: String,
    pub other_name: String,
    pub settings: Settings,
}

/// Mensajes que llegan a la ventana mientras se transcribe.
pub enum Event {
    Progress(Progress),
    Finished(Result<PathBuf>),
}

/// Lanza la transcripción en segundo plano y devuelve el canal por el que avisa.
pub fn run(job: Job) -> Receiver<Event> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let (ptx, prx) = channel::<Progress>();
        let forward = tx.clone();
        let forwarder = thread::spawn(move || {
            for p in prx {
                if forward.send(Event::Progress(p)).is_err() {
                    break;
                }
            }
        });
        let result = execute(&job, &ptx);
        drop(ptx);
        let _ = forwarder.join();
        let _ = tx.send(Event::Finished(result));
    });
    rx
}

fn execute(job: &Job, progress: &Sender<Progress>) -> Result<PathBuf> {
    let model_path = ensure_model(&job.settings.model, progress)?;

    let _ = progress.send(Progress::Status("Cargando el modelo…".into()));
    let model = Model::load(&model_path)?;

    let my_wav = job.dir.join(&job.settings.my_recording_file);
    let other_wav = job.dir.join(&job.settings.other_recording_file);

    let _ = progress.send(Progress::Status(format!("Transcribiendo a {}…", job.my_name)));
    let _ = progress.send(Progress::Percent(0));
    let mine = model.transcribe(&my_wav, &job.settings.language, &job.settings.vocabulary, progress)?;

    let _ = progress.send(Progress::Status(format!("Transcribiendo a {}…", job.other_name)));
    let _ = progress.send(Progress::Percent(0));
    let theirs = model.transcribe(&other_wav, &job.settings.language, &job.settings.vocabulary, progress)?;

    let _ = progress.send(Progress::Status("Escribiendo la transcripción…".into()));
    let markdown = to_markdown(
        &job.title,
        job.started_at,
        &[
            Speaker { name: &job.my_name, segments: &mine },
            Speaker { name: &job.other_name, segments: &theirs },
        ],
    );
    let out = job.dir.join(&job.settings.transcript_file);
    write_text(&out, &markdown)?;
    Ok(out)
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).with_context(|| format!("escribir {}", path.display()))
}
