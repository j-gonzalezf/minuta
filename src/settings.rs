//! Ajustes del programa, guardados en un archivo TOML legible en la carpeta de
//! configuración del usuario (`%APPDATA%\minuta\config.toml`).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Carpeta donde se crea una subcarpeta por reunión.
    pub base_dir: PathBuf,
    /// Cómo te llamas en la transcripción (pista del micrófono).
    pub my_name: String,
    /// Nombre por defecto de la otra persona (pista del altavoz); se puede cambiar en cada reunión.
    pub other_name: String,
    /// Nombres de los archivos dentro de la carpeta de la reunión.
    pub my_recording_file: String,
    pub other_recording_file: String,
    pub transcript_file: String,
    /// Idioma de la transcripción (código de dos letras de Whisper).
    pub language: String,
    /// Modelo de Whisper (nombre del archivo ggml sin extensión).
    pub model: String,
}

impl Default for Settings {
    fn default() -> Self {
        let documents = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            base_dir: documents.join("minuta"),
            my_name: "Jorge".to_string(),
            other_name: "Cliente".to_string(),
            my_recording_file: "Mi_grabacion.wav".to_string(),
            other_recording_file: "Grabacion_cliente.wav".to_string(),
            transcript_file: "transcripcion.md".to_string(),
            language: "es".to_string(),
            model: "ggml-large-v3-turbo".to_string(),
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("minuta")
            .join("config.toml")
    }

    /// Carga los ajustes; si no existen, devuelve los de serie (sin escribirlos).
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|err| {
                eprintln!("Ajustes ilegibles en {}: {err}. Se usan los de serie.", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("crear {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializar los ajustes")?;
        std::fs::write(&path, text).with_context(|| format!("escribir {}", path.display()))?;
        Ok(())
    }

    /// Carpeta donde se guardan los modelos de Whisper descargados.
    pub fn models_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("minuta")
            .join("models")
    }
}
