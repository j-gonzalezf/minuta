//! Prueba sin ventana: graba 5 segundos las dos pistas en una carpeta temporal
//! y muestra cuánto dura cada archivo y cuánta señal tiene.
//!
//! cargo run --example prueba_grabacion

#[path = "../src/audio.rs"]
mod audio;

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let dir = std::env::temp_dir().join("minuta-prueba");
    let mic = dir.join("mic.wav");
    let speaker = dir.join("altavoz.wav");
    println!("Grabando 5 segundos en {}", dir.display());
    let recorder = audio::Recorder::start(&mic, &speaker)?;
    for i in 1..=5 {
        std::thread::sleep(Duration::from_secs(1));
        println!("  {i} s (contador: {:.2} s)", recorder.recorded_seconds());
    }
    recorder.stop()?;
    for path in [mic, speaker] {
        let reader = hound::WavReader::open(&path)?;
        let spec = reader.spec();
        let samples: Vec<i16> = reader.into_samples::<i16>().collect::<Result<_, _>>()?;
        let seconds = samples.len() as f64 / spec.sample_rate as f64;
        let rms = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt();
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        println!("{}: {:.2} s, {} Hz, rms {:.0}, pico {}", path.file_name().unwrap().to_string_lossy(), seconds, spec.sample_rate, rms, peak);
    }
    Ok(())
}
