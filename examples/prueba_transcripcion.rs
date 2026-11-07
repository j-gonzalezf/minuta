//! Prueba sin ventana: transcribe dos WAV y muestra el Markdown resultante.
//!
//! cargo run --example prueba_transcripcion -- mi.wav otro.wav [Nombre1] [Nombre2]

#[path = "../src/merge.rs"]
mod merge;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/transcribe.rs"]
mod transcribe;

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mine = PathBuf::from(args.first().expect("falta el primer WAV"));
    let theirs = PathBuf::from(args.get(1).expect("falta el segundo WAV"));
    let my_name = args.get(2).cloned().unwrap_or_else(|| "Jorge".into());
    let other_name = args.get(3).cloned().unwrap_or_else(|| "Martín".into());
    let settings = settings::Settings::load();

    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for p in rx {
            match p {
                transcribe::Progress::Status(s) => eprintln!("· {s}"),
                transcribe::Progress::Percent(p) if p % 25 == 0 => eprintln!("  {p}%"),
                _ => {}
            }
        }
    });

    let t = Instant::now();
    let model_path = model::ensure_model(&settings.model, &tx)?;
    let model = transcribe::Model::load(&model_path)?;
    eprintln!("Modelo cargado en {:.1} s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let a = model.transcribe(&mine, &settings.language, &settings.vocabulary, &tx)?;
    let b = model.transcribe(&theirs, &settings.language, &settings.vocabulary, &tx)?;
    eprintln!("Transcritas dos pistas en {:.1} s ({} + {} frases)", t.elapsed().as_secs_f64(), a.len(), b.len());

    let md = merge::to_markdown(
        "Prueba",
        chrono::Local::now(),
        &[
            merge::Speaker { name: &my_name, segments: &a },
            merge::Speaker { name: &other_name, segments: &b },
        ],
    );
    println!("{md}");
    Ok(())
}
