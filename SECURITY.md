# Política de seguridad

Ecliven Code ejecuta comandos, edita archivos y lanza procesos externos en nombre del usuario. Tomamos en serio cualquier fallo que permita saltarse esos controles.

## Versiones soportadas

| Versión | Soporte de seguridad |
|---|---|
| Última versión publicada (`0.3.x`) | Sí |
| Versiones anteriores | No: actualiza con `ecliven update` |

## Cómo reportar una vulnerabilidad

**No abras un issue público.** Usa el reporte privado de GitHub:

[Reportar una vulnerabilidad](https://github.com/ikiresearch/ecliven-code/security/advisories/new)

Incluye, si puedes:

- versión (`ecliven --version`), sistema operativo y modo de sandbox (`read-only`, `workspace-write`, `danger-full-access`);
- pasos mínimos para reproducirlo y qué control se salta;
- impacto esperado (qué puede leer, escribir o ejecutar un atacante);
- una propuesta de corrección, si la tienes.

No incluyas API keys, tokens ni datos personales reales en el reporte.

## Plazos

| Etapa | Objetivo |
|---|---|
| Acuse de recibo | 3 días hábiles |
| Evaluación inicial y severidad | 7 días |
| Corrección de severidad crítica o alta | 30 días |
| Divulgación coordinada | Al publicar la corrección, o a los 90 días |

Te mantendremos informado del avance y te daremos crédito en el aviso de seguridad si lo deseas.

## Alcance

Dentro del alcance:

- escapar del sandbox de comandos (Landlock en Linux, Seatbelt en macOS) o escribir fuera de las rutas permitidas;
- saltarse Workspace Trust: que un `.ecliven.toml` no confiable active MCP, comandos, `base_url` de proveedores, `swarm.verify` o modos de aprobación;
- path traversal o symlinks que permitan leer o escribir fuera del workspace;
- ejecutar herramientas sin la aprobación que exige el modo configurado, o con la política `read-only` activa;
- fuga de API keys, tokens o variables de entorno (logs, sesiones guardadas, mensajes al modelo, servidores MCP);
- que un candidato de Swarm escriba fuera de su copia aislada o altere los tests con los que se le evalúa;
- integridad de los instaladores (`install.sh`, `install.ps1`) y de los binarios publicados.

Fuera del alcance:

- código incorrecto o inseguro generado por un modelo, si no se salta ningún control de Ecliven;
- acciones que el usuario aprobó de forma explícita, o el modo `danger-full-access`;
- la falta de aislamiento del sistema operativo en Windows, que ya está documentada (en `workspace-write` cada comando pide aprobación y en `read-only` se bloquean);
- vulnerabilidades de los CLI externos o proveedores de modelos que no dependan de Ecliven.

## Verificar un release

Cada release publica `SHA256SUMS.txt` y una atestación de procedencia de compilación:

```bash
sha256sum -c SHA256SUMS.txt --ignore-missing
gh attestation verify ecliven-x86_64-unknown-linux-musl.tar.gz --repo ikiresearch/ecliven-code
```

## Puerto seguro

No emprenderemos acciones contra quien investigue de buena fe, respete esta política, no acceda a datos de terceros, no degrade el servicio y nos dé tiempo razonable para corregir antes de divulgar.
