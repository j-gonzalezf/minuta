//! Grabación simultánea de dos pistas con WASAPI: el micrófono en uso (tu voz)
//! y el altavoz en uso en modo *loopback* (la voz de la otra persona).
//!
//! Las dos pistas se piden ya convertidas a 16 kHz mono en coma flotante, que
//! es lo que consume Whisper, y se escriben a WAV de 16 bits. Mientras está en
//! pausa se sigue leyendo del dispositivo (para no dejar que el búfer se
//! desborde) pero no se escribe nada.
//!
//! Detalle importante: la captura del altavoz solo entrega datos cuando algo
//! está sonando. Si nadie habla, no llegan paquetes y la pista se quedaría
//! corta y desalineada respecto al micrófono. Por eso un tercer hilo reproduce
//! silencio en el altavoz durante toda la grabación: así el *loopback* entrega
//! muestras de forma continua y las dos pistas miden lo mismo.

use anyhow::{Context, Result, anyhow};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

pub const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;

/// Qué pista es.
#[derive(Clone, Copy, Debug)]
pub enum Track {
    /// Micrófono por defecto.
    Mic,
    /// Altavoz por defecto, capturado en modo loopback.
    Speaker,
}

/// Una grabación en marcha. Al soltarla se detiene y se cierran los archivos.
pub struct Recorder {
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    threads: Vec<JoinHandle<()>>,
    pub mic_path: PathBuf,
    pub speaker_path: PathBuf,
}

impl Recorder {
    /// Empieza a grabar las dos pistas en los archivos indicados.
    pub fn start(mic_path: &Path, speaker_path: &Path) -> Result<Self> {
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));

        // Los archivos se abren aquí, en el hilo principal, para que un fallo
        // (carpeta inexistente, sin permisos) se vea antes de arrancar nada.
        let mic_writer = open_wav(mic_path)?;
        let speaker_writer = open_wav(speaker_path)?;

        // Cada hilo prepara su dispositivo (tarda hasta un segundo) y los tres
        // arrancan a la vez en la barrera, para que las dos pistas empiecen en el
        // mismo instante y queden alineadas.
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = Vec::new();
        threads.push(spawn_capture(Track::Mic, mic_writer, &paused, &stop, Some(&frames), &error, &barrier));
        threads.push(spawn_capture(Track::Speaker, speaker_writer, &paused, &stop, None, &error, &barrier));
        threads.push(spawn_silence(&stop, &error, &barrier));

        // Si algún hilo falló al arrancar, se nota enseguida.
        thread::sleep(std::time::Duration::from_millis(300));
        if let Some(msg) = error.lock().unwrap().clone() {
            stop.store(true, Ordering::SeqCst);
            for t in threads {
                let _ = t.join();
            }
            return Err(anyhow!(msg));
        }

        Ok(Self {
            paused,
            stop,
            frames,
            error,
            threads,
            mic_path: mic_path.to_path_buf(),
            speaker_path: speaker_path.to_path_buf(),
        })
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Segundos grabados (sin contar las pausas).
    pub fn recorded_seconds(&self) -> f64 {
        self.frames.load(Ordering::Relaxed) as f64 / SAMPLE_RATE as f64
    }

    /// Error de alguno de los hilos, si lo ha habido.
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    /// Detiene la grabación y cierra los archivos.
    pub fn stop(mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        match self.error.lock().unwrap().clone() {
            Some(msg) => Err(anyhow!(msg)),
            None => Ok(()),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

type Writer = hound::WavWriter<std::io::BufWriter<std::fs::File>>;

fn open_wav(path: &Path) -> Result<Writer> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("crear {}", parent.display()))?;
    }
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    hound::WavWriter::create(path, spec).with_context(|| format!("crear {}", path.display()))
}

fn record_error(error: &Mutex<Option<String>>, msg: String) {
    let mut slot = error.lock().unwrap();
    if slot.is_none() {
        *slot = Some(msg);
    }
}

fn spawn_capture(
    track: Track,
    writer: Writer,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    frames: Option<&Arc<AtomicU64>>,
    error: &Arc<Mutex<Option<String>>>,
    barrier: &Arc<Barrier>,
) -> JoinHandle<()> {
    let paused = Arc::clone(paused);
    let stop = Arc::clone(stop);
    let frames = frames.cloned();
    let error = Arc::clone(error);
    let barrier = Arc::clone(barrier);
    thread::spawn(move || {
        if let Err(err) = capture_loop(track, writer, &paused, &stop, frames.as_ref(), &barrier) {
            record_error(&error, format!("{}: {err:#}", track_label(track)));
            // Que los demás no se queden esperando en la barrera si este hilo ha fallado antes.
            barrier.wait();
        }
    })
}

fn track_label(track: Track) -> &'static str {
    match track {
        Track::Mic => "micrófono",
        Track::Speaker => "altavoz",
    }
}

fn capture_loop(
    track: Track,
    mut writer: Writer,
    paused: &AtomicBool,
    stop: &AtomicBool,
    frames: Option<&Arc<AtomicU64>>,
    barrier: &Barrier,
) -> Result<()> {
    initialize_mta().ok().context("inicializar COM en el hilo de captura")?;

    let enumerator = DeviceEnumerator::new().context("enumerar dispositivos de audio")?;
    // Para el loopback se abre el dispositivo de *reproducción* y se le pide capturar.
    let device_dir = match track {
        Track::Mic => Direction::Capture,
        Track::Speaker => Direction::Render,
    };
    let device = enumerator
        .get_default_device(device_dir)
        .with_context(|| format!("no hay {} por defecto", track_label(track)))?;
    let mut client = device.get_iaudioclient().context("abrir el dispositivo")?;

    let format = WaveFormat::new(32, 32, SampleType::Float, SAMPLE_RATE as usize, CHANNELS as usize, None);
    let blockalign = format.get_blockalign() as usize;
    let (default_period, _) = client.get_device_period().context("leer el periodo del dispositivo")?;
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: default_period };
    client
        .initialize_client(&format, Direction::Capture, mode, None)
        .context("inicializar la captura (¿formato no admitido?)")?;
    let event = client.set_get_eventhandle().context("crear el evento de captura")?;
    let capture = client.get_audiocaptureclient().context("obtener el cliente de captura")?;

    let mut queue: VecDeque<u8> = VecDeque::with_capacity(SAMPLE_RATE as usize * blockalign);
    barrier.wait();
    client.start_stream().context("arrancar la captura")?;

    while !stop.load(Ordering::SeqCst) {
        // Con 3 s sin datos no es un error: el altavoz en silencio no entrega paquetes
        // (el hilo de silencio lo evita, pero por si acaso).
        let _ = event.wait_for_event(3000);
        capture.read_from_device_to_deque(&mut queue).context("leer del dispositivo")?;

        if paused.load(Ordering::SeqCst) {
            queue.clear();
            continue;
        }
        let usable = queue.len() - queue.len() % blockalign;
        let bytes: Vec<u8> = queue.drain(..usable).collect();
        let mut count = 0u64;
        for chunk in bytes.chunks_exact(4) {
            let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(value).context("escribir en el WAV")?;
            count += 1;
        }
        if let Some(frames) = frames {
            frames.fetch_add(count, Ordering::Relaxed);
        }
    }

    client.stop_stream().ok();
    writer.finalize().context("cerrar el WAV")?;
    Ok(())
}

/// Reproduce silencio en el altavoz por defecto mientras dura la grabación, para
/// que la captura en loopback reciba datos de forma continua (ver módulo).
fn spawn_silence(stop: &Arc<AtomicBool>, error: &Arc<Mutex<Option<String>>>, barrier: &Arc<Barrier>) -> JoinHandle<()> {
    let stop = Arc::clone(stop);
    let error = Arc::clone(error);
    let barrier = Arc::clone(barrier);
    thread::spawn(move || {
        if let Err(err) = silence_loop(&stop, &barrier) {
            record_error(&error, format!("silencio en el altavoz: {err:#}"));
            barrier.wait();
        }
    })
}

fn silence_loop(stop: &AtomicBool, barrier: &Barrier) -> Result<()> {
    initialize_mta().ok().context("inicializar COM en el hilo de silencio")?;
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator.get_default_device(Direction::Render).context("no hay altavoz por defecto")?;
    let mut client = device.get_iaudioclient()?;
    let format = WaveFormat::new(32, 32, SampleType::Float, SAMPLE_RATE as usize, CHANNELS as usize, None);
    let blockalign = format.get_blockalign() as usize;
    let (default_period, _) = client.get_device_period()?;
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: default_period };
    client.initialize_client(&format, Direction::Render, mode, None)?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    barrier.wait();
    client.start_stream()?;
    while !stop.load(Ordering::SeqCst) {
        let free = client.get_available_space_in_frames()? as usize;
        if free > 0 {
            let mut zeros: VecDeque<u8> = std::iter::repeat_n(0u8, free * blockalign).collect();
            render.write_to_device_from_deque(free, &mut zeros, None)?;
        }
        let _ = event.wait_for_event(1000);
    }
    client.stop_stream().ok();
    Ok(())
}
