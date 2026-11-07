//! minuta: graba una reunión en dos pistas (micrófono y altavoz) y la
//! transcribe en local con quién dijo cada frase y a qué minuto.

// Sin consola negra detrás de la ventana en la versión final; en desarrollo se
// mantiene para ver los mensajes de error.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod settings;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("minuta")
            .with_inner_size([520.0, 360.0])
            .with_min_inner_size([420.0, 300.0]),
        ..Default::default()
    };
    eframe::run_native("minuta", options, Box::new(|cc| Ok(Box::new(app::MinutaApp::new(cc)))))
}
