# Instrucciones para trabajar en Elyvon Studio

## Compilación de Windows x64

- Compila Windows x64 **localmente**, con el target `x86_64-pc-windows-msvc`.
- Usa la toolchain nativa en Windows. Si el entorno es Linux o macOS, prepara una toolchain de compilación cruzada local compatible con Tauri y el formato de instalador solicitado.
- No sustituyas la compilación local por un workflow de GitHub Actions salvo que el usuario lo pida expresamente. Si falta una dependencia o la compilación cruzada no es viable, explica el bloqueo concreto.
- Verifica los binarios e instaladores locales antes de publicarlos en un release; conserva los requisitos de firma y los controles de versión del repositorio.
