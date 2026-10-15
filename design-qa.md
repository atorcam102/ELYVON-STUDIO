# Revisión del diseño aplicado a la aplicación principal

## Alcance

El diseño del frontend se ha aplicado al punto de entrada nativo `src/main.tsx` → `src/App.tsx`, sin sustituir la aplicación por la página `/reference.html`. Se conservan los estados de proyecto, comandos IPC, guardado, edición, mute, solo, volumen, paneles y preferencias existentes.

## Evidencia renderizada

- Referencia visual: las capturas aportadas en la conversación y el frontend de referencia conservado en `src/reference/`.
- Aplicación principal antes: `.local/native-before.png`.
- Aplicación principal después: `.local/native-final.png`.
- Biblioteca e inspector: `.local/native-library-inspector.png`.
- Escritorio compacto: `.local/native-1080.png` y `.local/native-960.png`.
- Navegador: Chromium real mediante Playwright, con el mock oficial de IPC de Tauri y un proyecto de prueba. No se sustituye el componente App ni se añade un mock al código de producción.
- Viewports: 1440 × 900, 1080 × 700 y 960 × 600 CSS px, densidad 1.

## Superficies comprobadas

- Tipografía: Arial/Helvetica, texto compacto de 8–11 px, LCD con reloj de 15 px y cifras tabulares.
- Composición: transporte gris claro, herramientas del arreglo, cabeceras de 44 px, biblioteca a la izquierda del inspector y del arreglo, mezclador con una sola barra superior.
- Colores: superficies grafito, regiones con el color real de cada pista, efectos azules, entradas de instrumento verdes y controles de pan verdes.
- Iconos: Phosphor en los componentes de la aplicación principal; se mantienen los nombres y las propiedades accesibles de los controles.
- Datos: nombres, colores, notas MIDI, niveles y comandos proceden del proyecto. Los slots FX siguen deshabilitados. Las ilustraciones de audio se limitan a las regiones sin archivo de sesiones demo y se rotulan como tales. No se simula audio real.

## Correcciones durante la revisión

- Se corrigió un desplazamiento de 12 px que recortaba la forma de onda ilustrativa.
- Se limitó la altura máxima del mezclador para mantener visible el arreglo a 960 × 600.
- Se compactaron los slots y las separaciones del mezclador en ventanas de hasta 720 px de alto.
- Se comprobó que la biblioteca, el inspector y el arreglo quedan ordenados de izquierda a derecha, en x = 0, 236 y 452 px en el caso de prueba.

## Validación funcional

- TypeScript, lint y compilación de producción: correctos.
- Vitest: 31 pruebas aprobadas.
- Verificado sobre la entrada principal: selección de pistas, comandos IPC de mute y solo, volumen por teclado, añadir pista desde Functions, cambio a piano roll y regreso al mezclador, apertura/cierre de biblioteca e inspector.
- Sin errores `pageerror` en Chromium.
- Los controles del transporte permanecen dentro de la ventana mínima de 960 px.
- No se ha ejecutado el binario Windows localmente; su compilación corresponde a CI.

## Límite de la comparación

La integración y el funcionamiento de los controles han sido comprobados. La equivalencia estricta píxel a píxel con las imágenes originales sigue sin certificarse: no se dispone aquí de sus archivos normalizados para una comparación conjunta a escala idéntica, y el contenido real del proyecto difiere de la sesión de las capturas.

**final result: blocked** — únicamente para certificar la igualdad visual exacta; los checks de integración anteriores pasan.
