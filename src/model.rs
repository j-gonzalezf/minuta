//! Descarga del modelo de Whisper (formato ggml de whisper.cpp) la primera vez.
//! Los modelos viven en `%LOCALAPPDATA%\minuta\models`.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::settings::Settings;
use crate::transcribe::Progress;

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Ruta local del modelo; lo descarga si no está.
pub fn ensure_model(name: &str, progress: &Sender<Progress>) -> Result<PathBuf> {
    let dir = Settings::models_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("crear {}", dir.display()))?;
    let file = format!("{name}.bin");
    let path = dir.join(&file);
    if path.exists() {
        return Ok(path);
    }

    let url = format!("{BASE_URL}/{file}");
    let _ = progress.send(Progress::Status(format!("Descargando el modelo {name} (solo la primera vez)…")));
    let response = ureq::get(&url).call().with_context(|| format!("descargar {url}"))?;
    if response.status().as_u16() != 200 {
        bail!("el servidor respondió {} al pedir {url}", response.status());
    }
    let total: Option<u64> = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());

    // Se escribe a un archivo temporal y se renombra al final: si la descarga se
    // corta, no queda un modelo a medias que luego falle al cargar.
    let tmp = dir.join(format!("{file}.descargando"));
    let mut out = std::fs::File::create(&tmp).with_context(|| format!("crear {}", tmp.display()))?;
    let mut reader = response.into_body().into_reader();
    let mut buf = vec![0u8; 1 << 20];
    let mut done: u64 = 0;
    let mut last_pct = u32::MAX;
    loop {
        let n = reader.read(&mut buf).context("leer la descarga")?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).context("escribir el modelo")?;
        done += n as u64;
        if let Some(total) = total {
            let pct = ((done * 100) / total.max(1)) as u32;
            if pct != last_pct {
                last_pct = pct;
                let _ = progress.send(Progress::Percent(pct));
            }
        }
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, &path).with_context(|| format!("renombrar a {}", path.display()))?;
    Ok(path)
}
