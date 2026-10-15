# Capturas de Elyvon Studio

Capturas del frontend principal del commit [`fbc4566`](https://github.com/atorcam102/ELYVON-STUDIO/commit/fbc456623aced9396cc235f6fd2f5d5697a1cd14), con el nuevo diseño aplicado a `src/App.tsx` y sus componentes. No proceden de la maqueta `/reference.html`.

| Archivo | Vista |
| --- | --- |
| `01-arrangement-mixer.png` | Arreglo de la sesión demo y mezclador. |
| `02-library-inspector.png` | Biblioteca e inspector con Keys seleccionada. |
| `03-piano-roll.png` | Vista de solo lectura de las notas de Chords. |

## Método de captura

- Chromium mediante Playwright, ventana de 1280 × 800 CSS px y densidad 1.
- Entrada principal de Vite: `/`, con los componentes de la aplicación Tauri.
- Puente IPC sustituido únicamente durante la captura por el mock oficial de `@tauri-apps/api/mocks`.
- Proyecto de muestra con las seis pistas, regiones, tempo y patrones MIDI definidos en `crates/elyvon-project/src/demo.rs`; se utilizan identificadores estables para capturar las vistas.
- Datos de carpetas de ejemplo en la biblioteca. No representan el disco de una instalación real.
- Motor marcado como no disponible y simulación de medidores desactivada.
- Las ilustraciones de audio son las del componente `DemoWaveform`, con su etiqueta de demostración. No hay archivos de audio detrás de esas regiones.

Son capturas reales del frontend renderizado, no imágenes generadas ni composiciones. No acreditan la ejecución del instalador Windows ni la existencia de reproducción, grabación o procesamiento de audio.

Para actualizar las imágenes, conserva los nombres de archivo y captura la misma entrada principal, revisando también las funciones y limitaciones descritas en el README raíz.
