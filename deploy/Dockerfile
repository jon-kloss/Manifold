# FICSIT Planner — web deploy image (Web Phase 4a, Railway target).
#
# Two stages. The builder needs ONLY Node/pnpm — NOT a Rust toolchain — because
# the wasm-bindgen package (renderer/src/wasm/web-pkg) is committed to the repo
# and CI gates it in sync with crates/web (`regen-web-wasm.sh check`). So the
# image builds the renderer bundle from the committed wasm via `build:web:static`
# (tsc + vite, no wasm-pack). The runtime stage is a tiny Caddy static server.
#
# Build context is the repo ROOT (see railway.toml / `docker build .`), so paths
# are repo-relative. .dockerignore keeps node_modules/dist/target out of context.

# ---- build ----
FROM node:22-alpine AS build
# Let corepack fetch the pinned pnpm non-interactively (no TTY in a build).
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
# Use the pnpm version pinned in renderer/package.json (packageManager field),
# so the image, CI, and local dev all build on the SAME pnpm — corepack reads
# that field once the package.json is copied. Pinning matters: pnpm's default
# blocking of dependency build scripts is a hard error outside a CI/interactive
# shell, and renderer/pnpm-workspace.yaml's `allowBuilds` (the pnpm-11 format)
# is what approves esbuild's build here.
RUN corepack enable
WORKDIR /app/renderer

# Install deps against the lockfile first (cached until the lockfile changes).
COPY renderer/package.json renderer/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile

# Then the renderer sources (includes the committed src/wasm/web-pkg) and build.
COPY renderer/ ./
RUN pnpm build:web:static

# ---- runtime ----
FROM caddy:2-alpine
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /app/renderer/dist-web /srv
# Railway sets $PORT and routes to it; 8080 is the local-run default (Caddyfile).
EXPOSE 8080
CMD ["caddy", "run", "--config", "/etc/caddy/Caddyfile", "--adapter", "caddyfile"]
