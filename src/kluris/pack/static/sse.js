// Tiny EventSource wrapper for the Kluris chat UI.
//
// Native EventSource does not support POST bodies — we need to send
// the user's message in the request body, so we use fetch + a manual
// SSE parser instead. Identical event shape: ``data: <json>`` followed
// by ``\n\n``; the terminator ``data: [DONE]`` ends the stream.

(function () {
  async function streamChat({ url, body, onEvent }) {
    const resp = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!resp.ok || !resp.body) {
      onEvent({ type: "error", message: "HTTP " + resp.status });
      return;
    }
    const reader = resp.body.getReader();
    const decoder = new TextDecoder("utf-8");
    let buf = "";
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      let idx;
      while ((idx = buf.indexOf("\n\n")) !== -1) {
        const frame = buf.slice(0, idx);
        buf = buf.slice(idx + 2);
        const line = frame.split("\n").find((l) => l.startsWith("data:"));
        if (!line) continue;
        const payload = line.slice(5).trim();
        if (payload === "[DONE]") return;
        try {
          onEvent(JSON.parse(payload));
        } catch (err) {
          // Skip malformed frames — keep the stream flowing.
        }
      }
    }
  }

  window.kluris = Object.assign(window.kluris || {}, { streamChat });
})();
