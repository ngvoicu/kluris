// Brain explorer — left-sidebar tree + click-to-expand modal.
//
// Talks to three read-only endpoints exposed by ``routes/chat.py``:
//   GET /api/brain/tree       — wake_up payload
//   GET /api/brain/neuron     — read_neuron payload (?path=…)
//   GET /api/brain/lobe       — lobe_overview payload (?lobe=…)
//
// All endpoints return the same shape the LLM agent sees through its
// retrieval tools, so the human-facing tree is a faithful reflection
// of what the agent would find.

(function () {
  const $ = (id) => document.getElementById(id);

  // ---- Markdown-ish renderer ---------------------------------------
  // The chat is intentionally plaintext-with-pre-wrap; the modal needs
  // a richer view for headings, lists, links, and inline code. We
  // implement a tiny Markdown subset (headings, lists, fenced code,
  // inline code, bold, italic, links) instead of pulling in a 50KB
  // markdown library. Anything we don't render falls through as text.

  function escapeHtml(s) {
    return s
      .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  }

  function renderInline(text) {
    let html = escapeHtml(text);
    // Inline code first so its contents don't get re-processed.
    html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
    // Bold + italic (very simple, no nesting).
    html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
    html = html.replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
    // Links.
    html = html.replace(
      /\[([^\]]+)\]\(([^)\s]+)\)/g,
      '<a class="md-link" href="#" data-md-link="$2">$1</a>'
    );
    return html;
  }

  function _isTableRow(line) {
    return /^\s*\|.*\|\s*$/.test(line);
  }
  function _isTableSeparator(line) {
    // GFM separator: `| --- | :---: | ---: |` etc. Must contain a dash.
    return /^\s*\|[\s:|\-]+\|\s*$/.test(line) && /-/.test(line);
  }
  function _parseRow(line) {
    let row = line.trim();
    if (row.startsWith("|")) row = row.slice(1);
    if (row.endsWith("|")) row = row.slice(0, -1);
    return row.split("|").map((c) => c.trim());
  }
  function _renderTable(headerLine, bodyLines) {
    const headers = _parseRow(headerLine);
    let html = "<table><thead><tr>";
    for (const h of headers) {
      html += "<th>" + renderInline(h) + "</th>";
    }
    html += "</tr></thead><tbody>";
    for (const row of bodyLines) {
      const cells = _parseRow(row);
      html += "<tr>";
      for (const c of cells) {
        html += "<td>" + renderInline(c) + "</td>";
      }
      html += "</tr>";
    }
    html += "</tbody></table>";
    return html;
  }

  function renderMarkdown(md) {
    const lines = (md || "").split(/\r?\n/);
    const out = [];
    let inCode = false;
    let codeBuf = [];
    let listType = null; // "ul" | "ol" | null
    let inQuote = false;
    function flushList() {
      if (listType) {
        out.push("</" + listType + ">");
        listType = null;
      }
    }
    function flushQuote() {
      if (inQuote) {
        out.push("</blockquote>");
        inQuote = false;
      }
    }
    let i = 0;
    while (i < lines.length) {
      const raw = lines[i];
      if (inCode) {
        if (/^```/.test(raw)) {
          out.push("<pre><code>" + escapeHtml(codeBuf.join("\n")) +
                   "</code></pre>");
          codeBuf = [];
          inCode = false;
        } else {
          codeBuf.push(raw);
        }
        i++; continue;
      }
      if (/^```/.test(raw)) {
        flushList(); flushQuote();
        inCode = true;
        i++; continue;
      }
      // Table: header row immediately followed by a separator row.
      if (
        _isTableRow(raw) &&
        i + 1 < lines.length &&
        _isTableSeparator(lines[i + 1])
      ) {
        flushList(); flushQuote();
        const header = raw;
        const body = [];
        i += 2;
        while (i < lines.length && _isTableRow(lines[i])) {
          body.push(lines[i]);
          i++;
        }
        out.push(_renderTable(header, body));
        continue;
      }
      const h = /^(#{1,6})\s+(.*)$/.exec(raw);
      if (h) {
        flushList(); flushQuote();
        const level = h[1].length;
        out.push("<h" + level + ">" + renderInline(h[2]) + "</h" + level + ">");
        i++; continue;
      }
      const ul = /^[-*]\s+(.*)$/.exec(raw);
      const ol = /^\d+\.\s+(.*)$/.exec(raw);
      if (ul) {
        flushQuote();
        if (listType !== "ul") { flushList(); out.push("<ul>"); listType = "ul"; }
        out.push("<li>" + renderInline(ul[1]) + "</li>");
        i++; continue;
      }
      if (ol) {
        flushQuote();
        if (listType !== "ol") { flushList(); out.push("<ol>"); listType = "ol"; }
        out.push("<li>" + renderInline(ol[1]) + "</li>");
        i++; continue;
      }
      const bq = /^>\s?(.*)$/.exec(raw);
      if (bq) {
        flushList();
        if (!inQuote) { out.push("<blockquote>"); inQuote = true; }
        out.push("<p>" + renderInline(bq[1]) + "</p>");
        i++; continue;
      }
      flushList(); flushQuote();
      if (/^\s*$/.test(raw)) {
        out.push("");
        i++; continue;
      }
      if (/^[-=]{3,}\s*$/.test(raw)) {
        out.push("<hr>");
        i++; continue;
      }
      out.push("<p>" + renderInline(raw) + "</p>");
      i++;
    }
    flushList(); flushQuote();
    if (inCode) {
      out.push("<pre><code>" + escapeHtml(codeBuf.join("\n")) +
               "</code></pre>");
    }
    return out.join("\n");
  }

  // ---- YAML preview ------------------------------------------------
  // Tiny line-based highlighter for ``.yml`` / ``.yaml`` neurons
  // (notably the OpenAPI specs in ``projects/*/openapi.yml``). Not a
  // YAML parser — regex-based coloring of keys, strings, numbers,
  // booleans, and comments. Pulling in a real highlighter would
  // outweigh the value here.
  function _highlightYamlLine(escapedLine) {
    if (/^\s*#/.test(escapedLine)) {
      return '<span class="yaml-comment">' + escapedLine + "</span>";
    }
    let html = escapedLine;
    // Trailing inline comment.
    html = html.replace(
      /(\s)(#.*)$/,
      '$1<span class="yaml-comment">$2</span>',
    );
    // Key at start of line. Allow simple slug, slash, dot, or quoted.
    html = html.replace(
      /^(\s*-?\s*)('[^']*'|"[^"]*"|[\w./\-]+)(\s*:)/,
      '$1<span class="yaml-key">$2</span>$3',
    );
    // Quoted strings (after the key has already been wrapped, so the
    // span attribute quotes are safe — they don't match `'` / `"`
    // inside a value position).
    html = html.replace(
      /(:\s|-\s)('[^']*'|"[^"]*")/g,
      '$1<span class="yaml-string">$2</span>',
    );
    // Booleans / null / numbers after `: ` or `- `.
    html = html.replace(
      /(:\s|-\s)(true|false|null|yes|no)\b/g,
      '$1<span class="yaml-bool">$2</span>',
    );
    html = html.replace(
      /(:\s|-\s)(-?\d+(?:\.\d+)?)\b/g,
      '$1<span class="yaml-num">$2</span>',
    );
    return html;
  }
  function renderYaml(text) {
    const lines = (text || "").split(/\r?\n/);
    const highlighted = lines
      .map((l) => _highlightYamlLine(escapeHtml(l)))
      .join("\n");
    return '<pre class="yaml-preview"><code>' + highlighted + "</code></pre>";
  }

  // ---- Tree rendering ----------------------------------------------

  function renderLobeNode(lobe) {
    const li = document.createElement("li");
    li.className = "tree-node tree-lobe";
    li.dataset.lobe = lobe.name;
    li.innerHTML =
      '<div class="tree-row tree-lobe-row">' +
        '<button class="tree-toggle" type="button"' +
                ' aria-label="Toggle lobe">▸</button>' +
        '<button class="tree-label tree-lobe-label" type="button">' +
          '<span class="lobe-name">' + escapeHtml(lobe.name) + '/</span>' +
          '<span class="lobe-count">' + lobe.neurons + '</span>' +
        '</button>' +
      '</div>' +
      '<ul class="tree tree-children" hidden></ul>';
    return li;
  }

  function attachLobeNeurons(lobeLi, neurons) {
    const ul = lobeLi.querySelector(".tree-children");
    ul.innerHTML = "";
    if (!neurons.length) {
      const empty = document.createElement("li");
      empty.className = "tree-empty";
      empty.textContent = "(no neurons)";
      ul.appendChild(empty);
      return;
    }
    for (const n of neurons) {
      const li = document.createElement("li");
      li.className = "tree-node tree-neuron";
      if (n.deprecated) li.classList.add("is-deprecated");
      li.dataset.path = n.path;
      const tail = n.path.split("/").slice(-1)[0];
      li.innerHTML =
        '<button class="tree-row tree-label tree-neuron-label" type="button">' +
          '<span class="neuron-name">' + escapeHtml(n.title || tail) + '</span>' +
          '<span class="neuron-path">' + escapeHtml(tail) + '</span>' +
        '</button>';
      ul.appendChild(li);
    }
  }

  function renderRecent(items) {
    const ul = $("tree-recent");
    ul.innerHTML = "";
    if (!items || !items.length) {
      const empty = document.createElement("li");
      empty.className = "tree-empty";
      empty.textContent = "No recent activity";
      ul.appendChild(empty);
      return;
    }
    for (const r of items) {
      const li = document.createElement("li");
      li.className = "tree-node tree-neuron tree-recent-row";
      li.dataset.path = r.path;
      li.innerHTML =
        '<button class="tree-row tree-label" type="button">' +
          '<span class="neuron-name">' + escapeHtml(r.path) + '</span>' +
          '<span class="neuron-date">' + escapeHtml(r.updated || "") + '</span>' +
        '</button>';
      ul.appendChild(li);
    }
  }

  function renderGlossary(items) {
    const ul = $("tree-glossary");
    const count = $("glossary-count");
    ul.innerHTML = "";
    count.textContent = items && items.length ? "(" + items.length + ")" : "";
    if (!items || !items.length) {
      const empty = document.createElement("li");
      empty.className = "tree-empty";
      empty.textContent = "Glossary is empty";
      ul.appendChild(empty);
      return;
    }
    for (const g of items) {
      const li = document.createElement("li");
      li.className = "tree-node tree-glossary-row";
      const term = (g.term || "").replace(/^\*\*|\*\*$/g, "");
      li.innerHTML =
        '<button class="tree-row tree-label tree-glossary-label" type="button">' +
          '<span class="glossary-term">' + escapeHtml(term) + '</span>' +
          '<span class="glossary-def">' + escapeHtml(g.definition || "") + '</span>' +
        '</button>';
      ul.appendChild(li);
    }
  }

  // ---- Path resolution --------------------------------------------

  // Brain-relative path of whatever the modal is currently showing.
  // Used as the base directory when an inline markdown link uses a
  // relative path like ``../projects/foo/bar.md``. Without this, the
  // link gets sent to the API as-is, escapes the brain root, and the
  // sandbox returns 400.
  let currentNeuronPath = null;

  function resolveBrainPath(baseDir, target) {
    // Resolve a posix-style relative path. ``baseDir`` is the
    // directory part of the currently-open neuron's brain-relative
    // path (no trailing slash, may be ``""``). ``target`` is the
    // raw href from a markdown link, including any ``./`` / ``../``
    // segments and an optional ``#anchor``.
    const noAnchor = String(target).split("#")[0];
    if (!noAnchor) return null;
    // Absolute paths inside the API: leading slash means "from the
    // brain root"; the API sandbox strips one leading slash anyway.
    if (noAnchor.startsWith("/")) {
      return noAnchor.replace(/^\/+/, "");
    }
    const baseSegs = baseDir ? baseDir.split("/").filter(Boolean) : [];
    const targetSegs = noAnchor.split("/");
    const out = baseSegs.slice();
    for (const seg of targetSegs) {
      if (seg === "" || seg === ".") continue;
      if (seg === "..") {
        if (out.length === 0) return null; // escapes brain root
        out.pop();
      } else {
        out.push(seg);
      }
    }
    return out.join("/");
  }

  // ---- Modal -------------------------------------------------------

  const modal = $("modal");
  const backdrop = $("modal-backdrop");

  // Navigation stack inside the modal. Each entry is one of:
  //   {kind: "neuron", arg: "domain/foo.md"}
  //   {kind: "lobe",   arg: "projects"}
  //   {kind: "glossary", term, def}
  // Entries are pushed by the show* functions and consumed by the
  // back button. Closing the modal clears the stack so re-opening
  // starts fresh.
  const modalHistory = [];
  function pushHistory(entry) {
    modalHistory.push(entry);
    refreshBackButton();
  }
  function refreshBackButton() {
    const btn = $("modal-back");
    if (btn) btn.hidden = modalHistory.length <= 1;
  }
  function modalBack() {
    if (modalHistory.length < 2) return;
    modalHistory.pop();              // current view
    const prev = modalHistory.pop(); // will be re-pushed by show*
    if (prev.kind === "neuron")        showNeuron(prev.arg);
    else if (prev.kind === "lobe")     showLobe(prev.arg);
    else if (prev.kind === "glossary") showGlossary(prev.term, prev.def);
  }

  function openModal({eyebrow, title, meta, tags, bodyHtml}) {
    $("modal-eyebrow").textContent = eyebrow || "";
    $("modal-title").textContent = title || "";
    $("modal-meta").textContent = meta || "";
    const tagWrap = $("modal-tags");
    tagWrap.innerHTML = "";
    if (tags && tags.length) {
      for (const t of tags) {
        const chip = document.createElement("span");
        chip.className = "tag-chip";
        chip.textContent = t;
        tagWrap.appendChild(chip);
      }
    }
    $("modal-body").innerHTML = bodyHtml || "";
    backdrop.hidden = false;
    if (typeof modal.showModal === "function") {
      modal.showModal();
    } else {
      modal.setAttribute("open", "");
    }
    modal.scrollTop = 0;
  }
  function closeModal() {
    backdrop.hidden = true;
    modalHistory.length = 0;
    refreshBackButton();
    if (typeof modal.close === "function") {
      modal.close();
    } else {
      modal.removeAttribute("open");
    }
  }

  $("modal-close").addEventListener("click", closeModal);
  $("modal-back").addEventListener("click", modalBack);
  backdrop.addEventListener("click", closeModal);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && modal.hasAttribute("open")) {
      closeModal();
    }
  });

  function _neuronErrorMessage(status, path) {
    if (status === 404) {
      // map.md is auto-generated by ``kluris dream``; if the source
      // neuron is linking to one that doesn't exist, dream wasn't
      // run before the brain was packed, OR the link is a typo.
      const isMap = /(^|\/)map\.md$/.test(path);
      if (isMap) {
        return (
          "<p class='modal-error'>This <code>map.md</code> doesn't " +
          "exist in the bundled brain — most likely <code>kluris " +
          "dream</code> wasn't run against the source brain before " +
          "it was packed. Run <code>kluris dream --brain " +
          "&lt;name&gt;</code> in the source repo to regenerate the " +
          "map.md files, then <code>kluris pack --force</code> and " +
          "<code>docker compose up --build</code>.</p>"
        );
      }
      return (
        "<p class='modal-error'>This path doesn't exist in the " +
        "brain. The source neuron's link is probably stale (renamed " +
        "or moved file) — fix the link in the source brain and " +
        "re-pack.</p>"
      );
    }
    if (status === 400) {
      return (
        "<p class='modal-error'>The brain sandbox rejected this " +
        "path. The link tried to escape the brain root via " +
        "<code>..</code> or use an absolute filesystem path. Fix " +
        "the link in the source neuron.</p>"
      );
    }
    if (status >= 500) {
      return (
        "<p class='modal-error'>Server error reading the neuron. " +
        "Check <code>docker compose logs</code> for details.</p>"
      );
    }
    return (
      "<p class='modal-error'>Couldn't load this neuron " +
      "(HTTP " + status + ").</p>"
    );
  }

  async function showNeuron(path) {
    pushHistory({kind: "neuron", arg: path});
    try {
      const resp = await fetch("/api/brain/neuron?path=" +
                                encodeURIComponent(path));
      if (!resp.ok) {
        // 404 on a map.md may actually be navigable as a lobe —
        // ``map.md`` exists at every lobe root once the brain has
        // been ``dream``-ed, but if the deployer skipped that step
        // we can still surface the lobe contents via lobe_overview.
        const lobeMatch = /^(.+?)\/map\.md$/.exec(path);
        if (resp.status === 404 && lobeMatch) {
          // Don't keep the failed neuron entry in the back stack —
          // the lobe view replaces it as the visible modal.
          modalHistory.pop();
          await showLobe(lobeMatch[1]);
          return;
        }
        openModal({
          eyebrow: path,
          title: resp.status === 404
            ? "Path not in brain"
            : "Couldn't load this neuron",
          meta: "HTTP " + resp.status,
          bodyHtml: _neuronErrorMessage(resp.status, path),
        });
        return;
      }
      const data = await resp.json();
      // Track the brain-relative path of the open neuron so inline
      // markdown links using ``./`` or ``../`` resolve correctly.
      currentNeuronPath = data.path || path;
      const meta = data.frontmatter || {};
      const updated = meta.updated || "";
      const created = meta.created || "";
      const tags = Array.isArray(meta.tags) ? meta.tags : [];
      const isYaml = /\.(ya?ml)$/i.test(data.path || path);
      const bodyHtml = isYaml
        ? renderYaml(data.body || "")
        : renderMarkdown(data.body || "");
      const eyebrow =
        (data.deprecated ? "DEPRECATED · " : "") + (data.path || path);
      let title;
      if (isYaml) {
        // YAML neurons don't carry a body H1; prefer the frontmatter
        // ``title`` (set by the brain author), fall back to filename.
        title = (typeof meta.title === "string" && meta.title.trim())
          ? meta.title.trim()
          : (data.path || path).split("/").slice(-1)[0];
      } else {
        const titleLine = (data.body || "").split(/\r?\n/).find(
          (l) => /^#\s+/.test(l)
        );
        title = titleLine
          ? titleLine.replace(/^#\s+/, "").trim()
          : path.split("/").slice(-1)[0];
      }
      openModal({
        eyebrow: eyebrow,
        title: title,
        meta: [
          updated ? "updated " + updated : "",
          created ? "created " + created : "",
        ].filter(Boolean).join(" · "),
        tags: tags,
        bodyHtml: bodyHtml,
      });
    } catch (err) {
      openModal({
        eyebrow: path,
        title: "Network error",
        meta: String(err),
        bodyHtml: "",
      });
    }
  }

  async function showLobe(name) {
    pushHistory({kind: "lobe", arg: name});
    try {
      const resp = await fetch("/api/brain/lobe?lobe=" +
                                encodeURIComponent(name));
      if (!resp.ok) {
        const body = resp.status === 404
          ? "<p class='modal-error'>No lobe named <code>" +
            escapeHtml(name) + "</code> in the bundled brain. " +
            "The link may be stale (lobe renamed or moved).</p>"
          : "<p class='modal-error'>Couldn't load lobe " +
            "(HTTP " + resp.status + ").</p>";
        openModal({
          eyebrow: name + "/",
          title: resp.status === 404 ? "Lobe not in brain" : "Couldn't load lobe",
          meta: "HTTP " + resp.status,
          bodyHtml: body,
        });
        return;
      }
      const data = await resp.json();
      // Anchor relative markdown links in ``map_body`` to the lobe
      // directory by pretending the modal is showing the lobe's
      // ``map.md``. Without this, ``[evaluation-cycle](./evaluation-cycle.md)``
      // inside ``domain/map.md`` would resolve to the brain root.
      const lobeRel = (data.lobe || name).replace(/\/+$/, "");
      currentNeuronPath = lobeRel + "/map.md";
      const tagUnion = Array.isArray(data.tag_union) ? data.tag_union : [];
      const neurons = Array.isArray(data.neurons) ? data.neurons : [];
      let bodyHtml = renderMarkdown(data.map_body || "");
      bodyHtml +=
        '<h2>Neurons (' + neurons.length + ')</h2>' +
        '<ul class="modal-neuron-list">' +
        neurons.map(function (n) {
          const cls = n.deprecated
            ? "modal-neuron is-deprecated"
            : "modal-neuron";
          return (
            '<li class="' + cls + '">' +
              '<a href="#" class="modal-neuron-link" data-path="' +
                escapeHtml(n.path) + '">' +
                '<span class="modal-neuron-title">' +
                  escapeHtml(n.title || n.path) +
                '</span>' +
                '<span class="modal-neuron-path">' +
                  escapeHtml(n.path) +
                '</span>' +
              '</a>' +
              (n.excerpt
                ? '<p class="modal-neuron-excerpt">' +
                    escapeHtml(n.excerpt) +
                  '</p>'
                : '') +
            '</li>'
          );
        }).join("") +
        '</ul>';
      openModal({
        eyebrow: name.toUpperCase() + "/",
        title: "Lobe overview",
        meta: neurons.length + " neurons" +
              (data.truncated ? " · truncated" : ""),
        tags: tagUnion,
        bodyHtml: bodyHtml,
      });
    } catch (err) {
      openModal({
        eyebrow: name + "/",
        title: "Network error",
        meta: String(err),
        bodyHtml: "",
      });
    }
  }

  function showGlossary(term, definition) {
    pushHistory({kind: "glossary", term: term, def: definition});
    openModal({
      eyebrow: "GLOSSARY",
      title: term,
      meta: "",
      tags: [],
      bodyHtml: "<p>" + escapeHtml(definition || "") + "</p>",
    });
  }

  // ---- Filter -------------------------------------------------------

  function applyFilter(query) {
    const q = (query || "").toLowerCase().trim();
    // Filter input lives in the right panel but matches across the
    // left-panel lobes tree AND the right-panel recent + glossary lists.
    // Match every `.tree-node` under any `.tree` list — that's what binds
    // the two panels together.
    document.querySelectorAll(".tree .tree-node").forEach((n) => {
      if (!q) { n.hidden = false; return; }
      const text = n.textContent.toLowerCase();
      n.hidden = !text.includes(q);
    });
    // If filter active, expand all lobe children so matching neurons show.
    if (q) {
      document.querySelectorAll(
        "#tree-lobes .tree-children"
      ).forEach((ul) => { ul.hidden = false; });
      document.querySelectorAll(
        "#tree-lobes .tree-toggle"
      ).forEach((btn) => { btn.textContent = "▾"; });
    }
  }

  // ---- Boot ---------------------------------------------------------

  async function boot() {
    let tree;
    try {
      const resp = await fetch("/api/brain/tree");
      if (!resp.ok) throw new Error("HTTP " + resp.status);
      tree = await resp.json();
    } catch (err) {
      $("tree-lobes").innerHTML =
        '<li class="tree-empty">Failed to load brain tree: ' +
        escapeHtml(String(err)) + '</li>';
      return;
    }

    const lobesUl = $("tree-lobes");
    lobesUl.innerHTML = "";
    const lobes = (tree.lobes || []).slice().sort(
      (a, b) => a.name.localeCompare(b.name)
    );
    for (const lobe of lobes) {
      const li = renderLobeNode(lobe);
      lobesUl.appendChild(li);
    }

    renderRecent(tree.recent || []);
    renderGlossary(tree.glossary || []);

    const stats = $("sidebar-stats");
    stats.textContent =
      (tree.total_neurons || 0) + " neurons · " +
      lobes.length + " lobes";

    // Lobe row interactions: toggle button vs label.
    lobesUl.addEventListener("click", async (event) => {
      const node = event.target.closest(".tree-node.tree-lobe");
      if (!node) return;

      if (event.target.closest(".tree-toggle")) {
        const ul = node.querySelector(".tree-children");
        const btn = node.querySelector(".tree-toggle");
        if (ul.hidden && !ul.dataset.loaded) {
          ul.innerHTML = '<li class="tree-empty">Loading…</li>';
          try {
            const resp = await fetch("/api/brain/lobe?lobe=" +
                                      encodeURIComponent(node.dataset.lobe));
            if (resp.ok) {
              const data = await resp.json();
              attachLobeNeurons(node, data.neurons || []);
              ul.dataset.loaded = "1";
            } else {
              ul.innerHTML =
                '<li class="tree-empty">Failed to load</li>';
            }
          } catch (err) {
            ul.innerHTML =
              '<li class="tree-empty">Failed to load</li>';
          }
        }
        ul.hidden = !ul.hidden;
        btn.textContent = ul.hidden ? "▸" : "▾";
        return;
      }

      const lobeLabel = event.target.closest(".tree-lobe-label");
      if (lobeLabel) {
        showLobe(node.dataset.lobe);
        return;
      }

      const neuronNode = event.target.closest(".tree-neuron");
      if (neuronNode && neuronNode.dataset.path) {
        showNeuron(neuronNode.dataset.path);
      }
    });

    // Recent row interactions.
    $("tree-recent").addEventListener("click", (event) => {
      const node = event.target.closest(".tree-neuron");
      if (node && node.dataset.path) {
        showNeuron(node.dataset.path);
      }
    });

    // Glossary row interactions.
    $("tree-glossary").addEventListener("click", (event) => {
      const node = event.target.closest(".tree-glossary-row");
      if (!node) return;
      const term = node.querySelector(".glossary-term").textContent;
      const def = node.querySelector(".glossary-def").textContent;
      showGlossary(term, def);
    });

    // In-modal markdown links: navigate to a neuron path if it's
    // brain-relative; ignore http(s) and #anchors. Relative paths
    // (``./foo``, ``../bar``) resolve against the directory of the
    // currently-open neuron, NOT the brain root — otherwise a link
    // like ``[BTB frontend](../projects/btb-frontend-core/overview.md)``
    // would escape the brain root and 400.
    $("modal-body").addEventListener("click", (event) => {
      const link = event.target.closest("[data-md-link]");
      if (!link) {
        const neuronLink = event.target.closest(".modal-neuron-link");
        if (neuronLink) {
          event.preventDefault();
          showNeuron(neuronLink.dataset.path);
        }
        return;
      }
      event.preventDefault();
      const target = link.dataset.mdLink;
      if (/^https?:/.test(target)) {
        window.open(target, "_blank", "noopener");
        return;
      }
      const noAnchor = target.split("#")[0];
      const baseDir = currentNeuronPath
        ? currentNeuronPath.split("/").slice(0, -1).join("/")
        : "";
      // Resolve relative paths against the open neuron's directory.
      // We resolve regardless of file extension so a bare directory
      // link like ``./projects/btb-backend-summon`` can also navigate.
      const resolved = resolveBrainPath(baseDir, target);
      if (!resolved) {
        openModal({
          eyebrow: target,
          title: "Link points outside the brain",
          meta: "",
          bodyHtml:
            "<p class='modal-error'>This link uses <code>..</code> " +
            "to escape the brain root. Fix the source neuron's " +
            "link to point at a brain-relative path.</p>",
        });
        return;
      }
      // ``foo.md`` / ``.yml`` / ``.yaml`` → neuron view.
      if (
        noAnchor.endsWith(".md") || noAnchor.endsWith(".yml") ||
        noAnchor.endsWith(".yaml")
      ) {
        showNeuron(resolved);
        return;
      }
      // Trailing-slash directory or bare lobe name → lobe view.
      const cleanedTrailing = resolved.replace(/\/$/, "");
      if (
        target.endsWith("/") ||
        // single segment that doesn't look like a file
        (!cleanedTrailing.includes("/") && !cleanedTrailing.includes("."))
      ) {
        showLobe(cleanedTrailing);
        return;
      }
      // Anything else (e.g. ``foo/bar`` with no extension): try lobe
      // first; if 404 the error message will explain.
      showLobe(cleanedTrailing);
    });

    // Filter input.
    $("tree-filter").addEventListener("input", (event) => {
      applyFilter(event.target.value);
    });
  }

  window.kluris = Object.assign(window.kluris || {}, {
    bootBrainExplorer: boot,
    // Exposed so the chat code can render assistant-message bodies
    // through the same Markdown subset used in the brain modal.
    // Single source of truth for "what counts as markdown" in this UI.
    renderMarkdown: renderMarkdown,
    escapeHtml: escapeHtml,
  });
})();
