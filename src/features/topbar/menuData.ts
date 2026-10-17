/** The application menus follow the supplied Spanish desktop reference. */
export const menuData: Record<string, string> = {
  "Elyvon Studio": `Acerca de Elyvon Studio|app.about
Más información sobre Elyvon Studio…
Información acerca de control remoto…
---
Ajustes|app.settings|>
Superficies de control||>
Comandos de teclado||>
---
Biblioteca de sonidos…|view.browser
---
Enviar opinión sobre Elyvon Studio
---
Servicios||>
---
Ocultar Elyvon Studio
Ocultar otras apps
!Mostrar todo
---
Salir de Elyvon Studio|app.quit|Ctrl+Q`,
  Archivo: `Nueva|file.new|Ctrl+N
Nuevo a partir de plantilla…
Abrir…|file.open|Ctrl+O
Abrir recientes||>
---
Cerrar
Cerrar proyecto
Guardar|file.save|Ctrl+S
Guardar como…|file.saveAs|Ctrl+Mayús+S
Guardar una copia como…
Guardar como plantilla…
Restaurar a||>
---
Alternativas del proyecto||>
Gestión de proyectos||>
Ajustes del proyecto|app.settings|>
---
!Ajustar página…
!Imprimir||Ctrl+P
---
Vídeo||>
---
Importar|file.importAudio|>
Exportar|file.exportRender|>
Bounce|file.exportRender|>
Compartir||>`,
  Edición: `Deshacer|edit.undo|Ctrl+Z
Rehacer|edit.redo|Ctrl+Mayús+Z
Historial Deshacer…
Borrar historial Deshacer
---
Cortar|edit.cut|Ctrl+X
Copiar|edit.copy|Ctrl+C
Pegar|edit.paste|Ctrl+V
Pegar y reemplazar||Ctrl+Mayús+V
Pegar en la posición original
Eliminar|edit.delete|Supr
Borrar y trasladar
Copiar la selección de pasajes a una escena de Live Loops
Seleccionar||>
Seleccionar pistas||>
---
Mostrar Flex Pitch/Time||Ctrl+F
---
Repetir||>
Duración||>
Div.|region.split|>
Bounce o unir||>
Trasladar||>
Acortar||>
Convertir||>
Alargar duración||>
---
Copiar eventos MIDI…
Eliminar eventos MIDI||>
Separar eventos MIDI||>
---
Cortar/insertar tiempo||>
---
Tempo||>
---
!Abrir en editor de samples externo
Abrir editor de tempo inteligente|view.smartTempo`,
  Pista: `Nuevas pistas…|track.newInstrument|Ctrl+Alt+N
Nueva pista de audio|track.newAudio|Ctrl+Alt+A
Nueva pista de instrumento de software|track.newInstrument|Ctrl+Alt+S
Nueva pista SI de Session Player…
Nueva pista MIDI externa|track.newMidi|Ctrl+Alt+X
Otros||>
Renombrar pista|track.rename
---
Eliminar pista|track.delete
Eliminar pistas sin utilizar
---
Crear Track Stack…
!Compactar pila
!Convertir el grupo de carpetas en grupo de suma
---
Pistas globales||>
Mostrar Alternativas de pista
Mostrar Pista de salida
---
!Reemplazar o doblar pista de percusión…
!Convertir pasajes en nueva pista de sampler…
---
Buscar y seleccionar pista…
Ocultar la pista seleccionada
Ocultar la pista no seleccionada
!Mostrar todas las pistas
Activar/desactivar vista oculta||H
---
Ordenar pistas por||>
Asignar color de pista…
Asignar icono de pista…
Configurar cabecera de pista…`,
  Navegar: `Ir a||>
---
Ir a la selección|navigate.selection
Fijar localizadores en la selección y activar ciclo
Fijar localizadores redondeados en la selección y activar ciclo
---
Adelantar localizadores la longitud del ciclo
!Retrasar localizadores la longitud del ciclo
---
Crear marcador|transport.addMarker
Otros||>
!Renombrar marcador
!Eliminar marcador en la posición del cursor de reproducción
---
Abrir lista de marcadores|view.markers
Abrir texto de marcador|view.notes
---
Opciones de botón de reproducción||>
Opciones de botón de detención||>
---
Fijar localizadores automáticamente||>`,
  Grabar: `Compás de entrada|transport.countIn|>
Ajustes del metrónomo…|transport.settings
---
✓ Usar parrilla musical
✓ Permitir inicio de pinchazo rápido
✓ Monitorización de entrada automática
Modo de monitorización de baja latencia
---
Opciones de botón de grabación||>
Grabaciones MIDI superpuestas||>
Grabaciones de audio superpuestas||>
Ajustes de grabación…|record.settings`,
  Mezcla: `Mostrar automatización||A
Crear automatización de pista||>
Convertir automatización||>
Eliminar automatización||>
---
Mover la automatización de pista con los pasajes||>
Activar acceso rápido a automatización
Seleccionar automáticamente parámetro de automatización en modo lectura
Ajustes de automatización…
---
Ajustes de grupo
✓ Grupos activos
Crear grupo
Etiquetas E/S…
Asignaciones de E/S…
Medición pre-fader
---
Buscar y añadir módulo…
Mastering Assistant…
---
Dolby Atmos…`,
  Visualización: `Mostrar Biblioteca|view.browser|Y
Mostrar Inspector|view.inspector|I
---
Mostrar Mezclador|view.mixer|X
Mostrar Smart Controls|view.smartControls|B
Mostrar Editor|view.pianoRoll|E
---
Mostrar Editores de lista|view.events|D
Mostrar Blocs de notas|view.notes
Mostrar Explorador de loops|view.loops|O
Mostrar Exploradores|view.files|F
---
Ocultar Barra de controles|view.controlbar
Personalizar barra de controles y pantalla…
---
Mostrar Barra de herramientas
Personalizar barra de herramientas…
---
!Mostrar Ventana de vídeo
Mostrar Colores
---
Usar pantalla completa|view.fullscreen`,
  Ventana: `Minimizar|window.minimize|Ctrl+M
Zoom|view.fullscreen
Rellenar
Centrar
---
Trasladar y redimensionar||>
Ventana a pantalla completa|view.fullscreen|>
---
!Eliminar ventana del grupo
Pasar por las ventanas
---
Traer todo al frente
---
Abrir ventana principal|view.main|Ctrl+1
Abrir mezclador|view.mixer|Ctrl+2
---
Abrir Smart Controls|view.smartControls|Ctrl+3
Abrir editor de teclado|view.pianoRoll|Ctrl+4
Abrir Editor de partituras|view.score|Ctrl+5
Abrir secuenciador de pasos
Abrir Editor de pasos
Abrir el editor de pistas de audio|view.audioEditor
!Abrir editor de archivos de audio||Ctrl+6
Abrir editor de tempo inteligente|view.smartTempo
---
Abrir lista de eventos|view.events|Ctrl+7
Abrir lista de compases|view.signature
Abrir audio del proyecto|view.files|Ctrl+8
---
Abrir ventana “Transformación MIDI”||Ctrl+9
---
Ventana flotante “Abrir Transporte”
Mostrar Ventana flotante de eventos
Mostrar Ventana flotante de inspector de pasaje
---
Mostrar Teclado
Mostrar Escritura musical
Mostrar Teclado de introducción por pasos
---
Ocultar Todas las ventanas de módulos||V
---
✓ Sin título - Pistas|view.main`,
  Ayuda: `Ayuda de Elyvon Studio|app.about
Comandos de teclado
---
Buscar actualizaciones…|app.checkUpdates`,
};
