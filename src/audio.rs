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
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use wasapi::{Device, DeviceEnumerator, Direction, Role, SampleType, StreamMode, WaveFormat, initialize_mta};

pub const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;

/// Nombres de los dispositivos disponibles de un tipo (micrófonos o altavoces).
pub fn list_devices(track: Track) -> Vec<String> {
    let _ = initialize_mta();
    let mut names = Vec::new();
    if let Ok(enumerator) = DeviceEnumerator::new()
        && let Ok(collection) = enumerator.get_device_collection(device_direction(track))
    {
        for i in 0..collection.get_nbr_devices().unwrap_or(0) {
            if let Ok(device) = collection.get_device_at_index(i)
                && let Ok(name) = device.get_friendlyname()
            {
                names.push(name);
            }
        }
    }
    names
}

fn device_direction(track: Track) -> Direction {
    match track {
        Track::Mic => Direction::Capture,
        // Para el loopback se abre el dispositivo de *reproducción* y se le pide capturar.
        Track::Speaker => Direction::Render,
    }
}

/// El dispositivo pedido por nombre o, si no hay nombre o no se encuentra, el que
/// Windows usa para llamadas (papel "comunicaciones": el mismo que Discord).
fn pick_device(enumerator: &DeviceEnumerator, track: Track, wanted: Option<&str>) -> Result<Device> {
    let direction = device_direction(track);
    if let Some(wanted) = wanted.filter(|w| !w.trim().is_empty()) {
        let collection = enumerator.get_device_collection(direction)?;
        for i in 0..collection.get_nbr_devices()? {
            let device = collection.get_device_at_index(i)?;
            if device.get_friendlyname().map(|n| n == wanted).unwrap_or(false) {
                return Ok(device);
            }
        }
        // Si el dispositivo guardado ya no está (auriculares apagados), se usa el predeterminado.
    }
    enumerator
        .get_default_device_for_role(direction, Role::Communications)
        .or_else(|_| enumerator.get_default_device(direction))
        .with_context(|| format!("no hay {} predeterminado", track_label(track)))
}

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
    /// Nombres de los dispositivos que se están grabando (micrófono, altavoz).
    pub device_names: (String, String),
}

/// Qué dispositivos usar; `None` es el predeterminado de Windows para llamadas.
#[derive(Default, Clone)]
pub struct DeviceChoice {
    pub mic: Option<String>,
    pub speaker: Option<String>,
}

impl Recorder {
    /// Empieza a grabar las dos pistas en los archivos indicados.
    pub fn start(mic_path: &Path, speaker_path: &Path, devices: &DeviceChoice) -> Result<Self> {
        let paused = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));

        // Se resuelven aquí los dispositivos para saber sus nombres y fallar pronto.
        let _ = initialize_mta();
        let enumerator = DeviceEnumerator::new().context("enumerar dispositivos de audio")?;
        let mic_name = pick_device(&enumerator, Track::Mic, devices.mic.as_deref())?.get_friendlyname()?;
        let speaker_name = pick_device(&enumerator, Track::Speaker, devices.speaker.as_deref())?.get_friendlyname()?;
        let devices = DeviceChoice { mic: Some(mic_name.clone()), speaker: Some(speaker_name.clone()) };

        // Los archivos se abren aquí, en el hilo principal, para que un fallo
        // (carpeta inexistente, sin permisos) se vea antes de arrancar nada.
        let mic_writer = open_wav(mic_path)?;
        let speaker_writer = open_wav(speaker_path)?;

        // Cada hilo prepara su dispositivo (tarda hasta un segundo) y los tres
        // arrancan a la vez en la barrera, para que las dos pistas empiecen en el
        // mismo instante y queden alineadas.
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = Vec::new();
        threads.push(spawn_capture(Track::Mic, devices.mic.clone(), mic_writer, &paused, &stop, Some(&frames), &error, &barrier));
        threads.push(spawn_capture(Track::Speaker, devices.speaker.clone(), speaker_writer, &paused, &stop, None, &error, &barrier));
        threads.push(spawn_silence(devices.speaker.clone(), &stop, &error, &barrier));

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
            device_names: (mic_name, speaker_name),
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

#[allow(clippy::too_many_arguments)]
fn spawn_capture(
    track: Track,
    device: Option<String>,
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
        if let Err(err) = capture_loop(track, device.as_deref(), writer, &paused, &stop, frames.as_ref(), &barrier) {
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
    device: Option<&str>,
    mut writer: Writer,
    paused: &AtomicBool,
    stop: &AtomicBool,
    frames: Option<&Arc<AtomicU64>>,
    barrier: &Barrier,
) -> Result<()> {
    initialize_mta().ok().context("inicializar COM en el hilo de captura")?;

    let enumerator = DeviceEnumerator::new().context("enumerar dispositivos de audio")?;
    let device = pick_device(&enumerator, track, device)?;
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
    let started = std::time::Instant::now();
    client.start_stream().context("arrancar la captura")?;

    // Cada dispositivo tarda un tiempo distinto en entregar su primer paquete (el
    // altavoz, más de un segundo). Para que las dos pistas midan lo mismo desde el
    // instante común de arranque, el hueco inicial se rellena con silencio.
    let mut first_data = true;

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
        if usable == 0 {
            continue;
        }
        let bytes: Vec<u8> = queue.drain(..usable).collect();
        let mut count = 0u64;
        if first_data {
            first_data = false;
            let chunk_frames = (usable / blockalign) as f64;
            let gap_frames = started.elapsed().as_secs_f64() * SAMPLE_RATE as f64 - chunk_frames;
            for _ in 0..gap_frames.max(0.0) as u64 {
                writer.write_sample(0i16).context("escribir en el WAV")?;
                count += 1;
            }
        }
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
fn spawn_silence(device: Option<String>, stop: &Arc<AtomicBool>, error: &Arc<Mutex<Option<String>>>, barrier: &Arc<Barrier>) -> JoinHandle<()> {
    let stop = Arc::clone(stop);
    let error = Arc::clone(error);
    let barrier = Arc::clone(barrier);
    thread::spawn(move || {
        if let Err(err) = silence_loop(device.as_deref(), &stop, &barrier) {
            record_error(&error, format!("silencio en el altavoz: {err:#}"));
            barrier.wait();
        }
    })
}

fn silence_loop(device: Option<&str>, stop: &AtomicBool, barrier: &Barrier) -> Result<()> {
    initialize_mta().ok().context("inicializar COM en el hilo de silencio")?;
    let enumerator = DeviceEnumerator::new()?;
    let device = pick_device(&enumerator, Track::Speaker, device)?;
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
