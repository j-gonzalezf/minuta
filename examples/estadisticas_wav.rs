//! Duración y nivel de un WAV, por tramos de 5 s. cargo run --example estadisticas_wav -- archivo.wav
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("falta el WAV");
    let reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<i16> = reader.into_samples::<i16>().collect::<Result<_, _>>()?;
    let rate = spec.sample_rate as usize;
    println!("{}: {:.1} s, {} Hz, {} canal(es)", path, samples.len() as f64 / rate as f64, rate, spec.channels);
    for (i, tramo) in samples.chunks(rate * 5).enumerate() {
        let rms = (tramo.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / tramo.len() as f64).sqrt();
        let pico = tramo.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        println!("  {:>3}-{:<3} s  rms {:>6.0}  pico {:>6}", i * 5, i * 5 + 5, rms, pico);
    }
    Ok(())
}
