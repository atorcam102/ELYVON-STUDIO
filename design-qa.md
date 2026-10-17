# Revisión visual — 17 de octubre de 2026

## Alcance y coordinación

Se han aplicado las referencias a la aplicación principal React/Tauri. La previsualización `?design-preview&region` usa una sesión de diseño solo en DEV; no simula un motor de audio conectado.

El rediseño quedó conservado en `72e8514` y fusionado con el backend en `86b9c13`. Claude confirmó en `.local/agent-coordination.md` que se ocupa de conectar funciones y de la comprobación nativa. Codex conserva CSS de referencia, AppMenu, iconos y QA. No se han restaurado archivos ni eliminado sus modificaciones.

## Comparación renderizada

Referencias: las 20 capturas originales aportadas por el usuario. Se compara su distribución teniendo en cuenta el estiramiento horizontal indicado; no se aplica una transformación que deforme la UI.

Evidencia en `.local/design-qa-2026-10-17/`:

- `mixer-comparison.png`: referencia y aplicación juntas a una escala de comparación aproximada de 1280 × 800. La referencia muestra un panel menos expandido y corta el extremo inferior de los canales.
- `mixer-expanded.png`: panel abierto al máximo; controles superiores visibles.
- `mixer-minimum.png`: mismo recorrido de fader con recorte superior.
- `library-inspector.png`, `inspector.png`, `events.png`, `files.png`, `piano.png`, `score.png`, `tempo.png`, `menu-edit.png`, `menu-window.png`: revisión de superficies del rediseño. Estas últimas capturas preceden a la conexión funcional final de Claude.

## Correcciones verificadas

- Faders de altura fija: fila de 208 px, recorrido efectivo de 181 px. No se comprimen al reducir el mezclador.
- Todos los canales anclados abajo, incluidos Deluxe Classic y Stereo Out. Sus controles se recortan por arriba de forma uniforme.
- A 1280 × 800, panel máximo de 620 px; ambos ajustes superiores quedan dentro del panel. No se reproduce el exceso de altura que impedía verlos.
- Arrastre: mínimo de 180 px; cierre tras otros 48 px hacia abajo. Comprobado con arrastre real; se limpia el estado de redimensionado al cerrar.
- El selector real de automatización ocupa el ancho y la altura del slot; mantiene colores diferenciados para Touch/Latch/Write.
- Herramientas Puntero/Marquesina/Ripple siguen accesibles desde el menú conectado por Claude. Se ocultaron sus controles duplicados, que desplazaban el zoom.
- Menús españoles con ancho acorde a las referencias, límites de ventana y desplazamiento vertical. Biblioteca, inspector, panel lateral, piano, partitura y tempo siguen el reparto de superficies de las capturas.

## Validación

- Vitest: 48/48 pruebas, 8 archivos, sobre la versión fusionada.
- Compilación de producción y lint: correctos después de que Claude retirase un import sobrante durante su edición. `git diff --check` sin errores de espacios.
- Las seis pruebas locales Windows x64 de preferencias pasaron antes de la fusión. No equivalen a verificar el motor o el binario de la nueva integración; Claude se encarga de esa comprobación.

## Límites pendientes

No se certifica igualdad píxel a píxel. Quedan diferencias de iconos, controles propios del motor, marca de la aplicación y datos de la sesión. Algunas funciones que el usuario pidió presentar aún no están implementadas y lo indican al activarlas. La partitura y el tempo están recibiendo datos reales por parte de Claude; su validación final nativa queda en su trabajo.

## Segunda pasada — editores y menús

Comparación conjunta inspeccionada en `piano-comparison.png` y `score-comparison.png`, con las referencias 5 y 6 normalizadas aproximadamente al estiramiento indicado. Capturas finales: `piano-final.png`, `score-final.png`, `tempo-refined.png`; prueba compacta: `editor-compact.png` (960 × 720).

- Se restauraron las tres pestañas principales, centradas y del mismo ancho. Automatización y Tomas/audio conservan accesos de icono con nombre accesible a la derecha.
- Barra del piano organizada por grupos, con herramientas en menú, zoom horizontal y vertical reales, controles MIDI y seguimiento del cursor. Se mantienen sus estados y callbacks de edición.
- Ajustados pesos tipográficos, espaciados, selector de escala, checkbox e iconos de la biblioteca/barra superior. El texto CONSERVAR ya no tiene un borde de botón ajeno a la referencia.
- Partitura: barra propia completa, tres botones de vista, claves mayores, más separación de pentagramas, resaltado limitado a los compases reales del pasaje, compás vacío representado y regla con subdivisiones. El canvas se redibuja al cambiar su altura. Se conserva scoreModel y el cálculo real de posición y altura de las notas.
- Tempo inteligente: barra simplificada y LCD centrado, sin botones superpuestos. Se conservan los valores reales del mapa de tempo y el aviso de análisis no disponible.
- Menús: apertura con flechas, cambio entre menús con izquierda/derecha, Home/End, cierre con Escape y foco correcto. Verificado en una ventana de 960 px, incluido el scroll al último elemento.

Verificación final: TypeScript y build correctos; ESLint sin errores; Vitest **50/50 en 9 archivos**; recarga limpia sin errores de consola. Probados Borrador/Lápiz, zoom vertical, pestañas principales y acceso a Automatización. Los archivos del backend y bindings de Claude no se han editado.

P3 restantes: iconos de biblioteca que aproximan los originales, diferencias de fuente entre macOS/Windows, la marca Elyvon, información real del proyecto y el aviso de notación simplificada. No se certifica identidad píxel a píxel de toda la aplicación.

**final result: passed** para esta iteración visual de editores, menús y conservación del mixer. La equivalencia 1:1 estricta de toda la aplicación sigue sin certificarse.
