//! Transcripción en local con whisper.cpp (`whisper-rs`): una pista de audio
//! entra, salen frases con su tiempo de inicio y fin.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::sync::mpsc::Sender;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Una frase transcrita, con sus tiempos en segundos desde el inicio de la pista.
#[derive(Debug, Clone)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Avisos de progreso para la ventana.
#[derive(Debug, Clone)]
pub enum Progress {
    /// Texto corto de lo que se está haciendo.
    Status(String),
    /// Porcentaje de 0 a 100 de la pista en curso.
    Percent(u32),
}

/// Modelo cargado en memoria. Cargarlo tarda unos segundos; se reutiliza para las dos pistas.
pub struct Model {
    ctx: WhisperContext,
}

impl Model {
    pub fn load(model_path: &Path) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);
        let path = model_path.to_str().context("ruta del modelo con caracteres raros")?;
        let ctx = WhisperContext::new_with_params(path, params)
            .with_context(|| format!("cargar el modelo {}", model_path.display()))?;
        Ok(Self { ctx })
    }

    /// Transcribe una pista WAV (16 kHz, mono, 16 bits, como las que graba minuta).
    ///
    /// La pista se trocea primero por silencios (ver [`speech_chunks`]) y cada
    /// trozo se transcribe por separado: Whisper sitúa mal las frases cuando hay
    /// silencios largos (es habitual en una pista donde solo habla una persona),
    /// y así cada frase lleva el tiempo real del trozo en que suena.
    pub fn transcribe(
        &self,
        wav_path: &Path,
        language: &str,
        vocabulary: &[String],
        progress: &Sender<Progress>,
    ) -> Result<Vec<Segment>> {
        let audio = read_wav_16k_mono(wav_path)?;
        let chunks = speech_chunks(&audio, SAMPLE_RATE);
        if chunks.is_empty() {
            // Pista en silencio: Whisper se inventa texto con audio vacío.
            return Ok(Vec::new());
        }

        // El "prompt" orienta a Whisper: idioma y estilo, las palabras habituales y
        // lo último que se dijo, para que cada trozo no empiece sin contexto.
        let mut base_prompt = String::from("Transcripción de una reunión en español, con puntuación.");
        if !vocabulary.is_empty() {
            base_prompt.push_str(" Palabras habituales: ");
            base_prompt.push_str(&vocabulary.join(", "));
            base_prompt.push('.');
        }

        let total: usize = chunks.iter().map(|(s, e)| e - s).sum();
        let mut done = 0usize;
        let mut segments: Vec<Segment> = Vec::new();
        let mut last_text = String::new();
        for (start, end) in chunks {
            let offset = start as f64 / SAMPLE_RATE as f64;
            let prompt = if last_text.is_empty() { base_prompt.clone() } else { format!("{base_prompt} {last_text}") };
            for mut seg in self.transcribe_chunk(&audio[start..end], language, &prompt)? {
                seg.start += offset;
                seg.end += offset;
                last_text = seg.text.clone();
                segments.push(seg);
            }
            done += end - start;
            let _ = progress.send(Progress::Percent(((done * 100) / total.max(1)) as u32));
        }
        Ok(segments)
    }

    fn transcribe_chunk(&self, audio: &[f32], language: &str, prompt: &str) -> Result<Vec<Segment>> {
        // Whisper necesita al menos un segundo de audio; se rellena con silencio.
        let mut audio = audio.to_vec();
        if audio.len() < SAMPLE_RATE as usize {
            audio.resize(SAMPLE_RATE as usize, 0.0);
        }

        let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
        params.set_language(Some(language));
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_n_threads(std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4));
        // Sin esto, Whisper pega varias frases en un segmento largo; con cortes por
        // palabra salen frases naturales con su tiempo.
        params.set_token_timestamps(true);
        params.set_max_len(90);
        params.set_split_on_word(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_initial_prompt(prompt);

        let mut state = self.ctx.create_state().context("crear el estado de Whisper")?;
        state.full(params, &audio).context("transcribir")?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment.to_str_lossy().map(|t| t.trim().to_string()).unwrap_or_default();
            if text.is_empty() || is_noise(&text) {
                continue;
            }
            segments.push(Segment {
                start: segment.start_timestamp() as f64 / 100.0,
                end: segment.end_timestamp() as f64 / 100.0,
                text,
            });
        }
        Ok(segments)
    }
}

const SAMPLE_RATE: u32 = 16_000;

/// Trozos con voz `(inicio, fin)` en muestras, separados por silencios.
///
/// Detector sencillo por energía: la pista se mira en ventanas de 20 ms; una
/// ventana "suena" si su nivel supera un umbral relativo al ruido de fondo. Las
/// ventanas que suenan se agrupan en trozos; un silencio de más de 1,2 s cierra el
/// trozo, y cada trozo se amplía 0,25 s por cada lado para no cortar sílabas.
/// Los trozos muy largos se parten en un silencio interior para no superar los
/// 30 s que Whisper procesa de una vez.
pub fn speech_chunks(audio: &[f32], rate: u32) -> Vec<(usize, usize)> {
    let frame = (rate / 50) as usize; // 20 ms
    if audio.len() < frame {
        return Vec::new();
    }
    let levels: Vec<f32> = audio
        .chunks(frame)
        .map(|w| (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt())
        .collect();

    // Ruido de fondo: el nivel por debajo del cual está el 20 % más silencioso.
    let mut sorted = levels.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let floor = sorted[sorted.len() / 5];
    let threshold = (floor * 4.0).max(0.004);

    // Una pausa de más de 1,2 s cierra el trozo: las pausas para pensar dentro de
    // una frase suelen ser más cortas; las de escuchar al otro, más largas.
    let max_silence = (1.2 * 50.0) as usize; // ventanas de 20 ms
    let padding = (0.25 * 50.0) as usize;
    let max_len = 28 * 50;

    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut current: Option<(usize, usize)> = None; // en ventanas
    let mut silent = 0usize;
    for (i, level) in levels.iter().enumerate() {
        if *level > threshold {
            silent = 0;
            match current {
                Some((s, _)) => current = Some((s, i + 1)),
                None => current = Some((i, i + 1)),
            }
        } else if let Some((s, e)) = current {
            silent += 1;
            if silent > max_silence {
                chunks.push((s, e));
                current = None;
                silent = 0;
            }
        }
        // Trozo demasiado largo: se cierra aquí aunque siga sonando.
        if let Some((s, e)) = current
            && e - s >= max_len
        {
            chunks.push((s, e));
            current = None;
            silent = 0;
        }
    }
    if let Some(c) = current {
        chunks.push(c);
    }

    // Ampliar y pasar a muestras, uniendo los que se solapen.
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (s, e) in chunks {
        let start = s.saturating_sub(padding) * frame;
        let end = ((e + padding) * frame).min(audio.len());
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => out.push((start, end)),
        }
    }
    out
}

/// Marcas que Whisper escribe cuando no hay habla ("[Música]", "(risas)"…).
fn is_noise(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// Lee un WAV y lo deja como muestras f32 mono a 16 kHz. Acepta también WAV de
/// otras frecuencias o estéreo (por si se transcribe una grabación ajena),
/// remuestreando de forma sencilla.
pub fn read_wav_16k_mono(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("abrir {}", path.display()))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => reader.samples::<i16>().map(|s| s.map(|v| v as f32 / i16::MAX as f32)).collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, 32) => reader.samples::<i32>().map(|s| s.map(|v| v as f32 / i32::MAX as f32)).collect::<Result<_, _>>()?,
        (hound::SampleFormat::Float, 32) => reader.samples::<f32>().collect::<Result<_, _>>()?,
        (format, bits) => bail!("formato de WAV no admitido: {format:?} de {bits} bits"),
    };

    // A mono: media de los canales.
    let mono: Vec<f32> = if channels == 1 {
        samples
    } else {
        samples.chunks(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect()
    };

    // A 16 kHz: interpolación lineal. Suficiente para voz.
    let rate = spec.sample_rate;
    if rate == 16_000 {
        return Ok(mono);
    }
    let ratio = rate as f64 / 16_000.0;
    let out_len = (mono.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono[idx.min(mono.len() - 1)];
        let b = mono[(idx + 1).min(mono.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    Ok(out)
}
