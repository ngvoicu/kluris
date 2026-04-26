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
cp .env.example .env       # then edit .env with your credentials
docker compose up
# open http://localhost:8765
```

The chat is reachable as soon as `/healthz` returns 200. If it doesn't
boot, run `docker compose logs` — the smoke-test prints a single
redacted line naming the bad/missing env var.

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
