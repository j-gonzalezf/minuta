//! Intercala las frases de las dos pistas por tiempo y escribe el Markdown.

use crate::transcribe::Segment;
use chrono::{DateTime, Local};

pub struct Speaker<'a> {
    pub name: &'a str,
    pub segments: &'a [Segment],
}

/// Si entre dos frases seguidas de la misma persona hay menos de este tiempo,
/// van en la misma línea: Whisper parte las frases en cada pausa para pensar.
const JOIN_GAP_SECONDS: f64 = 2.0;

/// `[hh:mm:ss] Nombre: frase`, una por línea, en orden de tiempo. Las frases
/// seguidas de la misma persona se juntan en un párrafo para que se lea mejor,
/// y las que van casi pegadas, en la misma línea.
pub fn to_markdown(title: &str, started_at: DateTime<Local>, speakers: &[Speaker<'_>]) -> String {
    let mut raw: Vec<(f64, f64, &str, &str)> = Vec::new();
    for speaker in speakers {
        for seg in speaker.segments {
            raw.push((seg.start, seg.end, speaker.name, seg.text.as_str()));
        }
    }
    raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Unir frases casi pegadas de la misma persona, siempre que nadie hable en medio.
    let mut lines: Vec<(f64, &str, String)> = Vec::new();
    let mut last_end: Option<(f64, &str)> = None;
    for (start, end, name, text) in raw {
        match (lines.last_mut(), last_end) {
            (Some(last), Some((prev_end, prev_name))) if prev_name == name && start - prev_end < JOIN_GAP_SECONDS => {
                last.2.push(' ');
                last.2.push_str(text);
            }
            _ => lines.push((start, name, text.to_string())),
        }
        last_end = Some((end, name));
    }

    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!(
        "> **Fecha:** {} · **Asistentes:** {} · Transcripción automática de minuta: cada línea lleva la hora desde el inicio de la grabación.\n\n",
        started_at.format("%Y-%m-%d %H:%M"),
        speakers.iter().map(|s| s.name).collect::<Vec<_>>().join(" y ")
    ));
    out.push_str("---\n\n");

    let mut last_speaker: Option<&str> = None;
    for (start, name, text) in &lines {
        let name = *name;
        if last_speaker == Some(name) {
            out.push_str(&format!("[{}] {}\n", timestamp(*start), text));
        } else {
            if last_speaker.is_some() {
                out.push('\n');
            }
            out.push_str(&format!("**{name}**\n[{}] {}\n", timestamp(*start), text));
            last_speaker = Some(name);
        }
    }
    out
}

pub fn timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intercala_por_tiempo_y_agrupa_por_persona() {
        let jorge = vec![
            Segment { start: 0.0, end: 2.0, text: "Hola, ¿qué tal?".into() },
            Segment { start: 10.0, end: 12.0, text: "Vamos con la newsletter.".into() },
            Segment { start: 12.5, end: 14.0, text: "Primero el alta.".into() },
            Segment { start: 20.0, end: 21.0, text: "Y luego la baja.".into() },
        ];
        let martin = vec![Segment { start: 3.0, end: 5.0, text: "Bien, bien.".into() }];
        let md = to_markdown(
            "Prueba",
            Local::now(),
            &[Speaker { name: "Jorge", segments: &jorge }, Speaker { name: "Martín", segments: &martin }],
        );
        let body = md.split("---\n\n").nth(1).unwrap();
        assert_eq!(
            body,
            "**Jorge**\n[00:00:00] Hola, ¿qué tal?\n\n**Martín**\n[00:00:03] Bien, bien.\n\n**Jorge**\n[00:00:10] Vamos con la newsletter. Primero el alta.\n[00:00:20] Y luego la baja.\n"
        );
    }
}
