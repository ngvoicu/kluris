# {brain_name} — Kluris Pack

Self-contained Docker chat server for the **{brain_name}** brain.
The brain is baked into the image read-only; only LLM credentials
need to be supplied at deploy time.

## Prerequisites

- Docker (and Docker Compose v2)
- An LLM endpoint and credentials (Anthropic, OpenAI, or any
  OpenAI-compatible / Anthropic-compatible internal gateway)

## Local run

```bash
cp .env.example .env              # then edit .env with your credentials
docker compose up --build         # build image + start container
# open http://localhost:8765
```

`--build` is recommended on first launch and any time the brain
changes (the brain is baked into the image, so a brain edit needs a
rebuild before it shows up). On subsequent launches with no changes,
plain `docker compose up` is fine — Compose reuses the cached image.

### Re-packing after brain edits

When you edit the brain repo and want to refresh this pack directory:

```bash
kluris pack --brain {brain_name} --force --output <this-dir>
docker compose up --build
```

`--force` wipes and rebuilds the pack but **preserves `.env`,
`.env.local`, `.env.production`, and `.env.staging`** so your typed-in
LLM credentials survive across rebuilds. Everything else (Dockerfile,
compose, brain content, app code) is regenerated from the current
kluris install.

The chat is reachable as soon as `/healthz` returns 200. If it
doesn't boot, run `docker compose logs` — the smoke-test prints a
single redacted line naming the bad/missing env var.

### Corporate / on-prem LLM gateway TLS

If your gateway uses a self-signed cert or a certificate signed by a
private internal CA, you have two options (both in `.env.example`):

- `KLURIS_CA_BUNDLE=/data/corp-root-ca.pem` — point at a PEM bundle
  that includes your private root CA. Mount the file as a volume in
  `docker-compose.yml` if it lives outside the image:
  ```yaml
  volumes:
    - kluris-data:/data
    - /etc/corp/root-ca.pem:/data/corp-root-ca.pem:ro
  ```
- `KLURIS_TLS_INSECURE=1` — disables verification entirely. Opt-in
  only — boot prints a loud warning. Prefer the bundle path.

## Push to a registry

```bash
KLURIS_IMAGE=ghcr.io/me/{brain_name}:v1 docker compose build
docker push ghcr.io/me/{brain_name}:v1
```

The same Dockerfile produces both the local-only image and the
registry image — image tag = brain version.

## Cross-build for amd64 from M-series Mac

```bash
docker buildx build --platform linux/amd64 -t {brain_name}:v1 .
```

## Brain size

Keep the brain under ~50 MB for fast builds. The brain is baked into
the image, so accidentally-committed PDFs/images bloat layers and
slow `docker compose up --build` cycles.

## Public exposure

By default the compose file binds to `127.0.0.1:8765`. To expose
publicly, edit the port mapping AND put Kluris behind external
access control — reverse proxy auth, VPN, cloud IAM, or private
network. There is no built-in UI auth.

## Credential rotation

Edit `.env`, then `docker compose down && docker compose up`. Tokens
never persist to disk (OAuth bearer is in-memory only).
