# Revisión de fidelidad visual

**final result: blocked**

La implementación del frontend compila y las comprobaciones de navegador pasan. La verificación de igualdad exacta con las referencias sigue bloqueada: las capturas adjuntas se pueden consultar en la conversación, pero no están disponibles como archivos originales en el entorno para normalizar el encuadre y hacer una comparación conjunta a la misma escala. Este informe no certifica una réplica píxel a píxel.

## Referencias y evidencia

- Fuente visual: las cinco capturas adjuntas al mensaje del usuario. La cuarta es la referencia principal del mezclador y la disposición de pistas; la primera, del secuenciador y Live Loops; la tercera, de los instrumentos y acordes; la quinta, de la biblioteca.
- Capturas de implementación: `.local/studio-desktop.png`, `.local/studio-1080.png`, `.local/studio-library.png`, `.local/studio-loops.png`, `.local/studio-chords.png`, `.local/studio-small.png`.
- Navegador: Chromium, mediante Playwright, renderizado real. Escritorio 1440 × 900 y 1080 × 700 CSS px, densidad 1. Pantalla estrecha 768 × 900, densidad 1, con desplazamiento de la disposición de escritorio.
- Estados: mezclador, biblioteca abierta, Live Loops y secuenciador, instrumentos y selector de acordes.
- No se ha normalizado el bisel del dispositivo presente en las referencias. No se ha producido una comparación conjunta con los originales; la inspección de las capturas de implementación no sustituye ese paso.

## Superficies revisadas

- Tipografía: Arial/Helvetica/sans-serif, tamaños compactos de 8–11 px para pistas y canales, 15 px para el transporte. Falta verificar métricas frente a la fuente nativa de macOS de las referencias.
- Espaciado: composición de escritorio con transporte superior, pistas y mezclador; veinte canales visibles a 1080 px. Las cabeceras y regiones están alineadas en sus posiciones y alturas.
- Colores: transporte gris claro, superficies gris carbón, pistas azul, cian y violeta, efectos azules, instrumentos verdes y medidores verdes. Los valores se han inferido visualmente; no se han muestreado de los originales.
- Iconos y gráficos: iconos Phosphor; formas de onda y vistas MIDI dibujadas como visualizaciones de datos de muestra estáticos. No se utilizan imágenes de una captura como sustituto de la interfaz. Los iconos de instrumentos y los patrones de muestra difieren de los nativos de las referencias.
- Contenido: etiquetas de pistas, efectos, grupos, canales, biblioteca y acordes basadas en las capturas. Los datos de señal y volumen no proceden de una sesión de audio.

## Historial de correcciones

1. Cabeceras de pistas recortadas a 1080 × 700: se eliminó la altura mínima que desalineaba las filas. Comprobación posterior en navegador: diferencias de posición y altura entre cabecera y región = 0 px.
2. Medidores demasiado estrechos con veinte canales: se fijó su ancho y se redujo el ancho de los elementos de fader. Comprobación posterior: medidor de al menos 6 px, veinte canales sin desbordamiento del mezclador a 1080 px.
3. Proporciones de la vista principal: se redujo el ancho de la lista de pistas y se retiró la barra de título adicional para acercar la composición a la referencia principal.
4. Live Loops: se corrigió el tamaño mínimo del área de formas de onda que ocultaba la cuadrícula MIDI y la fila de bajo. Captura posterior: `.local/studio-loops.png` con las tres filas visibles.
5. Vista de instrumentos: se redujo la amplitud visual de las dos formas de onda de piano y se añadieron las notas MIDI del bajo. Captura posterior: `.local/studio-chords.png`.

## Comprobaciones realizadas

- `npm run build`: TypeScript y compilación de producción correctos.
- Cinco pistas, veinte canales de mezclador, 320 celdas del secuenciador y cuatro pistas de instrumentos presentes.
- Apertura y cierre de biblioteca, cambio entre pistas, Live Loops y acordes comprobados.
- Cambio de estado visual de mute comprobado. Los faders únicamente modifican su control local.
- Sin errores JavaScript registrados por `pageerror` durante las comprobaciones.
- `git diff --check`: correcto.

## Pendiente para cerrar la fidelidad exacta

- Comparar los originales y los renders normalizados juntos, incluida una comparación ampliada de transporte, canales e iconos.
- Ajustar las métricas tipográficas, los iconos y los datos visuales específicos que esa comparación determine.
- No se añaden funciones de audio, servicios o persistencia: el alcance es exclusivamente frontend.
