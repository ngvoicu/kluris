# {brain_name} — Kluris Pack

Self-contained Docker chat server for the **{brain_name}** brain.
The brain is baked into the image read-only; only the LLM connection
needs to be supplied at deploy time. There is no login — read
[Public exposure](#public-exposure) before putting it on a network.

## Prerequisites

- Docker (and Docker Compose v2)
- An LLM endpoint, reached with one of three setups (see
  [Configuration](#configuration)): an Anthropic API key, an OpenAI API
  key or OpenAI-compatible gateway, or an OAuth 2.0 client-credentials
  gateway that speaks the OpenAI Chat Completions API

## Local run

```bash
cp .env.example .env              # then edit .env with your LLM settings
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
kluris install — so re-apply any edit you made to `docker-compose.yml`
(for example a `127.0.0.1` port binding).

### If the chat doesn't come up

`/healthz` returns 200 as soon as the server process is up, so it does
not prove that chat works. A missing or invalid setting in `.env`, or a
failed start-up probe of your LLM endpoint, does not stop the
container: it starts in **brain-only mode** — the brain explorer works,
chat is disabled and answers with HTTP 503. Run `docker compose logs`
and look for the `kluris-pack:` line that names the problem (the
missing or conflicting variable, or the probe failure; credentials are
redacted). Fix `.env`, then restart.

## Configuration

Settings are environment variables in `.env`. The server reads them
once at boot, so after editing run
`docker compose down && docker compose up -d`. `.env.example` documents
every setting; boolean settings take `1`/`0` (or `true`/`false`).

### LLM connection (required)

The server reaches the LLM through a single LiteLLM-based provider.
Configure exactly one of these setups — setting variables from both the
API-key and the OAuth setups is a conflict and leaves chat disabled.

| Setup | Variables | Notes |
|-------|-----------|-------|
| Anthropic | `KLURIS_PROVIDER_SHAPE=anthropic`, `KLURIS_BASE_URL`, `KLURIS_API_KEY`, `KLURIS_MODEL` | The shape `.env.example` ships, pointing at `https://api.anthropic.com`. |
| OpenAI or an OpenAI-compatible gateway | `KLURIS_PROVIDER_SHAPE=openai`, `KLURIS_BASE_URL`, `KLURIS_API_KEY`, `KLURIS_MODEL` | Give the host root only (for example `https://api.openai.com`); the endpoint path is added for you, and a pasted `/v1/chat/completions` or `/v1/messages` suffix is trimmed with a boot warning. `api.openai.com` is called through OpenAI's Responses API; any other host through Chat Completions. |
| OAuth 2.0 client credentials | `KLURIS_OAUTH_TOKEN_URL`, `KLURIS_OAUTH_API_BASE_URL`, `KLURIS_OAUTH_CLIENT_ID`, `KLURIS_OAUTH_CLIENT_SECRET`, `KLURIS_MODEL`; optional `KLURIS_OAUTH_SCOPE` | For an internal gateway that speaks OpenAI Chat Completions. The bearer token is kept in memory only. |

### Tuning (optional)

| Variable | Default | What it does |
|----------|---------|--------------|
| `MAX_AGENT_ROUNDS` | `20` in code; `.env.example` sets `12` | Tool-calling rounds per answer; `0` means unlimited. The biggest lever on token cost. (No `KLURIS_` prefix.) |
| `KLURIS_MAX_TOOL_CALLS` | `0` (off) | Hard cap on the total tool calls in one answer. |
| `KLURIS_MAX_OUTPUT_TOKENS` | `4096` | Cap on the length of one response (16–200000). |
| `KLURIS_TEMPERATURE` | unset (the model's own default) | Sampling temperature, 0.0–2.0. |
| `KLURIS_REASONING_EFFORT` | unset | `none`, `minimal`, `low`, `medium`, `high` or `xhigh`, for models that take it. Applies to the OpenAI shape and the OAuth setup; the Anthropic shape ignores it and prints a boot warning. |
| `KLURIS_MAX_CONTEXT_TOKENS` | `24000` | Budget for the conversation history replayed each turn; the oldest turns are dropped. `0` means unlimited. |
| `KLURIS_MAX_TURN_TOKENS` | `96000` | Budget for the request within one answer; the oldest tool results are elided. `0` means unlimited. |
| `KLURIS_KEEP_RESULT_ROUNDS` | `3` | Tool results older than this many rounds are elided from the request. |
| `KLURIS_MAX_NEURON_BYTES` | `16384` | Cap on how much of one neuron the agent reads. `0` means unlimited. |
| `KLURIS_SESSION_RETENTION_DAYS` | `0` (keep forever) | Delete chat sessions older than this many days. |
| `KLURIS_LOCK_SYSTEM_PROMPT` | off | Read the system prompt once and keep it for the life of the process. |
| `KLURIS_SKIP_BOOT_SMOKE` | off | Skip the start-up probe of the LLM endpoint. |
| `KLURIS_DEBUG_STREAM` | off | Log one summary line per LLM stream to `docker compose logs`. |

`.env.example` also covers `KLURIS_ANTHROPIC_VERSION`,
`KLURIS_LOBE_OVERVIEW_BUDGET`, `KLURIS_MAX_MULTI_READ_PATHS` and
`KLURIS_USE_RESPONSES_API`. A value that is not a number, or is not an
accepted choice, disables chat the same way a missing setting does.

### Data and the system prompt

- **The brain** is baked into the image at build time, read-only, and
  cannot change while the container runs. Nothing in `.env` moves or
  unlocks it; to ship new brain content, re-pack and rebuild.
- **`/data`** is the `kluris-data` volume and the only writable path. It
  holds the chat history (`sessions.db`), the search index cache
  (`cache/`) and the system prompt. It survives `docker compose down`;
  `docker compose down -v` deletes it.
- **The system prompt** is the biggest behavior lever: it decides how the
  agent searches, cites sources and when it stops.
  `/data/config/system_prompt.md` is created from the bundled default on
  the first chat message and is **re-read on every request**, so edits
  take effect without a restart. With `KLURIS_LOCK_SYSTEM_PROMPT=1` it is
  read once and kept for the life of the process, so edits then need a
  restart.

### Corporate / on-prem LLM gateway TLS

If your gateway uses a self-signed cert or a certificate signed by a
private internal CA, you have two options (both in `.env.example`; set
at most one — setting both leaves chat disabled):

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
registry image — you choose the tag (for example your brain version).

## Cross-build for amd64 from M-series Mac

```bash
docker buildx build --platform linux/amd64 -t {brain_name}:v1 .
```

## Brain size

Keep the brain under ~50 MB for fast builds. The brain is baked into
the image, so accidentally-committed PDFs/images bloat layers and
slow `docker compose up --build` cycles.

## Public exposure

By default the compose file binds the port on all interfaces
(`8765:8765`). The server has NO built-in auth or rate limiting —
anyone who can reach the port can chat, read everything in the bundled
brain and read the saved conversations — so run it on a trusted host or
network. Restrict the mapping to `127.0.0.1:8765:8765` for local-only
access, and if you expose it more widely, put external access control
in front (reverse proxy auth, VPN, cloud IAM, or a private network).

## Credential rotation

Edit `.env`, then `docker compose down && docker compose up`. Tokens
never persist to disk (OAuth bearer is in-memory only).
