# minuta

Graba una reunión por videollamada en dos pistas, la tuya y la de la otra
persona, y la convierte en una transcripción en Markdown con quién dijo cada
frase y a qué minuto. Uso personal, Windows, sin internet y coste cero.

## Por qué

Las transcripciones automáticas de las reuniones no distinguen quién habla, y
luego hay que adivinarlo por contexto. Si tu voz entra por el micrófono y la de
la otra persona sale por el altavoz, grabar las dos fuentes por separado
resuelve el problema de raíz: cada pista tiene un único dueño.

## Qué hace

1. Al abrir, pides una ficha mínima: nombre de la reunión (por defecto la fecha)
   y quién está al otro lado.
2. **Grabar** captura a la vez el micrófono en uso y el altavoz en uso, cada uno
   en su archivo. **Pausar** deja de grabar sin cerrar la sesión. **Detener**
   termina.
3. Al detener, transcribe las dos pistas en local con Whisper, intercala las
   frases por tiempo y escribe la transcripción:

   ```
   [00:12:34] Jorge: …
   [00:12:41] Martín: …
   ```

Todo queda en una carpeta por reunión: `Mi_grabacion.wav`,
`Grabacion_cliente.wav` y `transcripcion.md`. Los nombres se cambian en ajustes.

## Estado

En construcción. Nada usable todavía.

## Stack

| Capa | Elección | Motivo |
|---|---|---|
| Lenguaje | Rust | Un solo ejecutable, rápido, sin instalar nada en el PC de destino |
| Ventana | `eframe` / `egui` | Interfaz sencilla en Rust puro: una ficha, tres botones y una barra de progreso. Sin pila web |
| Audio | `wasapi` + `hound` | Captura nativa de Windows, incluida la del altavoz (loopback), escrita a WAV |
| Transcripción | `whisper-rs` (whisper.cpp) con CUDA | La implementación más rápida de Whisper en local; usa la gráfica NVIDIA si la hay y la CPU si no |
| Ajustes | `serde` + `toml` | Un archivo de texto legible en la carpeta del usuario |

## Decisiones de diseño

- **Transcribir al terminar, no en directo.** Con el audio completo el modelo
  tiene contexto y acierta más. Grabar es trivial; transcribir una hora tarda
  uno o dos minutos con gráfica.
- **La frase es la unidad.** Whisper devuelve frases con su segundo de inicio;
  se intercalan las dos pistas por ese tiempo. No se redacta ni se resume: eso
  se hace después, leyendo la transcripción.
- **Las grabaciones se guardan siempre.** Si una transcripción sale mal o
  aparece un modelo mejor, se repite sin repetir la reunión.
- **Dos pistas, dos personas.** El programa no separa voces dentro de una
  pista: asume que por el micrófono hablas tú y por el altavoz la otra persona.
  Con altavoces en vez de cascos, el micrófono recoge también al otro; usa
  cascos.

## Desarrollo

Requisitos en Windows: Rust (`rustup`), Visual Studio Build Tools con la carga
de trabajo de C++, CMake, LLVM (para los enlaces a whisper.cpp) y, para usar la
gráfica, CUDA Toolkit.

```bash
cargo run            # compila y abre la ventana
cargo build --release
```

La primera compilación tarda varios minutos: construye whisper.cpp.

## Ramas

- `main` — versiones usables.
- `develop` — desarrollo.
