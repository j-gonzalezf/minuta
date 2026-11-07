//! Intercala las frases de las dos pistas por tiempo y escribe el Markdown.

use crate::transcribe::Segment;
use chrono::{DateTime, Local};

pub struct Speaker<'a> {
    pub name: &'a str,
    pub segments: &'a [Segment],
}

/// `[hh:mm:ss] Nombre: frase`, una por línea, en orden de tiempo. Las frases
/// seguidas de la misma persona se juntan en un párrafo para que se lea mejor.
pub fn to_markdown(title: &str, started_at: DateTime<Local>, speakers: &[Speaker<'_>]) -> String {
    let mut lines: Vec<(f64, &str, &str)> = Vec::new();
    for speaker in speakers {
        for seg in speaker.segments {
            lines.push((seg.start, speaker.name, seg.text.as_str()));
        }
    }
    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!(
        "> **Fecha:** {} · **Asistentes:** {} · Transcripción automática de minuta: cada línea lleva la hora desde el inicio de la grabación.\n\n",
        started_at.format("%Y-%m-%d %H:%M"),
        speakers.iter().map(|s| s.name).collect::<Vec<_>>().join(" y ")
    ));
    out.push_str("---\n\n");

    let mut last_speaker: Option<&str> = None;
    for (start, name, text) in lines {
        if last_speaker == Some(name) {
            out.push_str(&format!("[{}] {}\n", timestamp(start), text));
        } else {
            if last_speaker.is_some() {
                out.push('\n');
            }
            out.push_str(&format!("**{name}**\n[{}] {}\n", timestamp(start), text));
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
            "**Jorge**\n[00:00:00] Hola, ¿qué tal?\n\n**Martín**\n[00:00:03] Bien, bien.\n\n**Jorge**\n[00:00:10] Vamos con la newsletter.\n[00:00:12] Primero el alta.\n"
        );
    }
}
