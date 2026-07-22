# Security Policy

MANIFOLD is a local-first planning tool: plans stay on your machine (or in
your browser's storage), save files are parsed locally, and the only network
call the app can make is the optional AI provider you configure yourself.
Still, if you find a security issue — e.g. in `.sav` parsing, `Docs.json`
parsing, the web build's storage handling, or the release pipeline — please
report it privately.

## Reporting a vulnerability

- Use GitHub's **private vulnerability reporting**:
  [Security → Report a vulnerability](https://github.com/jon-kloss/Conveyancer/security/advisories/new).
- Please do **not** open a public issue for suspected vulnerabilities.
- Include reproduction steps and, if the issue involves a crafted file
  (`.sav` / `Docs.json`), attach or link the file.

You can expect an acknowledgement within a few days. Fixes ship as a normal
release (`MANIFOLD.exe` + the web deploy) once verified.

## Scope notes

- The desktop exe is unsigned; Windows SmartScreen warning on first run is
  expected and not a vulnerability.
- The bundled fixture catalog and vendored community assets are described in
  `NOTICE.md`.
