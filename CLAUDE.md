# Instrucciones para trabajar en Elyvon Studio

## Compilación, plataformas y releases

- Por defecto, compila **solo Windows x64**, con el target `x86_64-pc-windows-msvc`. Compila otros sistemas operativos o arquitecturas únicamente cuando el usuario lo pida expresamente.
- Compila siempre **localmente en el portátil del usuario**. Un entorno cloud, servidor o runner remoto no cuenta como su portátil. Si no tienes acceso al portátil o falta la toolchain necesaria, explica el bloqueo; no sustituyas el lugar de compilación por tu cuenta.
- **No uses GitHub Actions para ninguna tarea** (compilación, pruebas, publicación u otra automatización) salvo petición expresa del usuario. No lo uses como alternativa automática a una compilación local que falle.
- Evita activar Actions como efecto secundario de un push: revisa los triggers, usa `[skip ci]` en los commits de este flujo y no subas tags que disparen workflows sin petición expresa. No inicies ni reejecutes workflows automáticamente.
- Para entregar una versión descargable, verifica los binarios e instaladores compilados en el portátil y sube **esos mismos archivos a GitHub Releases**. No los recompiles en remoto ni los sustituyas por artefactos de CI. Publica sus hashes SHA-256 para poder comprobar la descarga.
- Conserva los requisitos de firma y los controles de versión del repositorio. Describe con precisión la plataforma compilada y cualquier limitación de validación.
