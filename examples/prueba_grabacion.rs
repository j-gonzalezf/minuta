//! Prueba sin ventana: graba unos segundos las dos pistas en una carpeta temporal
//! y muestra cuánto dura cada archivo y cuánta señal tiene.
//!
//! cargo run --example prueba_grabacion [-- segundos ["nombre del micrófono"]]

#[path = "../src/audio.rs"]
mod audio;

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(5);
    let devices = audio::DeviceChoice { mic: args.get(1).cloned(), speaker: None };

    let dir = std::env::temp_dir().join("minuta-prueba");
    let mic = dir.join("mic.wav");
    let speaker = dir.join("altavoz.wav");
    let recorder = audio::Recorder::start(&mic, &speaker, &devices)?;
    println!("Grabando {seconds} s en {}", dir.display());
    println!("  micrófono: {}\n  altavoz:   {}", recorder.device_names.0, recorder.device_names.1);
    for i in 1..=seconds {
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
