# Edit Recipes

Surgical edit recipes for `SPEC.html`. Every common operation has a before/after snippet. Match `old_string` precisely (with whitespace) and use the Edit tool.

After every edit, run the validate recipe (`references/validate.md`).

---

## Mark a task completed

```html
<!-- Before -->
<li class="task" id="task-INV-05" data-task="INV-05" data-status="pending">
```
```html
<!-- After -->
<li class="task" id="task-INV-05" data-task="INV-05" data-status="completed">
```

The visual checkbox flips to green automatically (CSS-only). No other edits needed — progress strings update on next page load.

---

## Mark a task blocked

```html
<!-- Before -->
<li class="task" id="task-INV-05" data-task="INV-05" data-status="pending">
```
```html
<!-- After -->
<li class="task" id="task-INV-05" data-task="INV-05" data-status="blocked">
```

If the whole phase is blocked, also change the phase: `data-status="in-progress"` → `data-status="blocked"`.

---

## Transition phase from pending to in-progress

When the previous phase completes and the next one starts:

```html
<!-- Before -->
<details class="phase" open data-phase="2" data-status="pending">
  <summary class="phase__header">
    <span class="phase__index">Phase 2</span>
    <h3 class="phase__title">Invite Flow</h3>
    <span class="pill pill--pending">Pending</span>
```
```html
<!-- After -->
<details class="phase" open data-phase="2" data-status="in-progress">
  <summary class="phase__header">
    <span class="phase__index">Phase 2</span>
    <h3 class="phase__title">Invite Flow</h3>
    <span class="pill pill--in-progress">In progress</span>
```

Two changes: `data-status` attribute AND the visible pill class+text.

---

## Transition phase to completed

```html
<!-- Before -->
<details class="phase" open data-phase="1" data-status="in-progress">
  ...
    <span class="pill pill--in-progress">In progress</span>
```
```html
<!-- After -->
<details class="phase" open data-phase="1" data-status="completed">
  ...
    <span class="pill pill--completed">Completed</span>
```

---

## Add a new task to a phase

Insert as the last `<li>` inside the phase's `<ul class="task-list">`. Task code = next number after the highest task in the spec (numbers are continuous across all phases, not per-phase).

The checkbox is **CSS-generated** — do NOT add a `<span class="task__check">` in markup. Required children are `.task__code` and `.task__text`; `.task__tags` is optional. The `data-status` on the `<li>` drives the checkbox state (pending / completed / blocked).

```html
<!-- Before -->
              <li class="task" id="task-INV-07" data-task="INV-07" data-status="pending">
                <span class="task__code">INV-07</span>
                <span class="task__text">Last existing task</span>
              </li>
            </ul>
```
```html
<!-- After -->
              <li class="task" id="task-INV-07" data-task="INV-07" data-status="pending">
                <span class="task__code">INV-07</span>
                <span class="task__text">Last existing task</span>
              </li>
              <li class="task" id="task-INV-08" data-task="INV-08" data-status="pending">
                <span class="task__code">INV-08</span>
                <span class="task__text">New task description with <code>file.ts</code> ref</span>
                <span class="task__tags"><span class="task__tag">#tag</span></span>
              </li>
            </ul>
```

---

## Add a new phase

Insert a new `<details class="phase">` block inside `<div class="phases">`. Phase number is the next integer.

```html
<!-- Before -->
        </details>
      </div>
    </section>
```
```html
<!-- After -->
        </details>

        <details class="phase" open data-phase="5" data-status="pending">
          <summary class="phase__header">
            <span class="phase__index">Phase 5</span>
            <h3 class="phase__title">Phase title</h3>
            <span class="pill pill--pending">Pending</span>
            <span class="phase__progress"><span data-progress-target="phase-5-total">0/0</span></span>
          </summary>
          <div class="phase__body">
            <ul class="task-list">
              <li class="task" id="task-INV-09" data-task="INV-09" data-status="pending">
                <span class="task__code">INV-09</span>
                <span class="task__text">Task description</span>
              </li>
            </ul>
          </div>
        </details>

      </div>
    </section>
```

---

## Check an acceptance criterion

```html
<!-- Before -->
<li class="ac-item" id="ac-3" data-ac="3" data-status="pending">
```
```html
<!-- After -->
<li class="ac-item" id="ac-3" data-ac="3" data-status="completed">
```

---

## Add a new acceptance criterion

Insert at the end of `<ul class="ac-list">` with the next `ac-N` id.

The checkbox is **CSS-generated** — do NOT add a `<span class="ac-check">` in markup. The only required child of `.ac-item` is `<span class="ac-text">`. The `data-status` on the `<li>` drives both the checkbox state and the strikethrough.

```html
<!-- Before -->
        <li class="ac-item" id="ac-5" data-ac="5" data-status="pending">
          <span class="ac-text">Last existing AC</span>
        </li>
      </ul>
```
```html
<!-- After -->
        <li class="ac-item" id="ac-5" data-ac="5" data-status="pending">
          <span class="ac-text">Last existing AC</span>
        </li>
        <li class="ac-item" id="ac-6" data-ac="6" data-status="pending">
          <span class="ac-text">New acceptance criterion text</span>
        </li>
      </ul>
```

For "Needs clarification" flag, wrap the text:

```html
<span class="ac-text"><span class="ac-flag">Needs clarification</span>The criterion text</span>
```

---

## Add a decision log entry

Append a `<tr>` to the Decision Log table's `<tbody>`.

```html
<!-- Before -->
        </tbody>
      </table>
    </section>
    <!-- endregion:decisions -->
```
```html
<!-- After -->
          <tr>
            <td class="log-table__date">2026-05-12</td>
            <td>The decision in 5-10 words</td>
            <td>The rationale in 1-2 sentences.</td>
          </tr>
        </tbody>
      </table>
    </section>
    <!-- endregion:decisions -->
```

---

## Add a deviation

```html
<!-- Before -->
        </tbody>
      </table>
    </section>
    <!-- endregion:deviations -->
```
```html
<!-- After -->
          <tr>
            <td><code>INV-05</code></td>
            <td>Spec said X</td>
            <td>Actually did Y</td>
            <td>Reason for the change.</td>
          </tr>
        </tbody>
      </table>
    </section>
    <!-- endregion:deviations -->
```

---

## Add a code-diff figure

Inside `<section id="code">`. Use unified format by default; add `data-view="split"` only for >30 line changes or multi-hunk.

```html
<figure class="code-diff" data-file="src/path/file.ts" data-language="typescript">
  <figcaption>
    <span><code>src/path/file.ts</code></span>
    <span class="code-diff__meta">+12 / -4</span>
  </figcaption>
<pre class="language-diff-typescript diff-highlight"><code>  function foo() {
-   return 1;
+   return 2;
  }</code></pre>
</figure>
```

`-` lines render with red background, `+` lines with green, both syntax-highlighted by PrismJS `diff-highlight` plugin.

### CRITICAL: escape HTML inside `<pre><code>`

`<pre><code>` is **not** a raw-text element in HTML5. Any `<`, `>`, or `&` inside the code body is parsed as markup. Unescaped Java/TS generics (`List<Object>`), JSX tags (`<Foo />`), shell redirects (`>>`), or `&&` operators silently corrupt the DOM — the browser swallows everything up to the next matching close tag, dropping later figures and entire sections. The validator reports this as `region/endregion mismatch — unclosed: [code]`.

Inside every `<pre><code>…</code></pre>` body, replace:

| Char | Entity |
|---|---|
| `&` | `&amp;` (do this first, otherwise you double-escape the entities below) |
| `<` | `&lt;` |
| `>` | `&gt;` |

**Wrong** (corrupts the DOM — `<Object>` and `<String, String>` become bogus tags):

```html
<pre class="language-diff-java diff-highlight"><code>     List<Object> args = new ArrayList<>();
-    Map<String, String> macros = repo.find(site, locale);
+    return repo.find(site, locale);
</code></pre>
```

**Right** (entity-escaped, renders identically, parses safely):

```html
<pre class="language-diff-java diff-highlight"><code>     List&lt;Object&gt; args = new ArrayList&lt;&gt;();
-    Map&lt;String, String&gt; macros = repo.find(site, locale);
+    return repo.find(site, locale);
</code></pre>
```

The `-` / `+` diff prefixes, whitespace, and PrismJS highlighting are unaffected by the escaping. Only the body text is escaped — leave `<pre>`, `<code>`, and their attributes alone.

---

## Add a wireframe mockup

Inside `<section id="mockups">`. Use `mockup--wireframe` class. Compose with `.wf-*` primitives — see `wireframe-library.md` for ready patterns.

```html
<figure class="mockup mockup--wireframe">
  <figcaption>
    <span><strong>Wireframe</strong> &middot; Screen name</span>
    <span class="code-diff__meta">structural — design TBD</span>
  </figcaption>
  <div class="mockup__body">
    <div class="wf-frame">
      <!-- wireframe content using .wf-* primitives -->
    </div>
  </div>
</figure>
```

---

## Add a hi-fi mockup

Use `mockup--hifi` class. Compose with `.ui-*` components — see `mockup-library.md` for ready patterns.

```html
<figure class="mockup mockup--hifi">
  <figcaption>
    <span><strong>Hi-fi</strong> &middot; Screen name</span>
  </figcaption>
  <div class="mockup__chrome">
    <span class="mockup__dot mockup__dot--red"></span>
    <span class="mockup__dot mockup__dot--yellow"></span>
    <span class="mockup__dot mockup__dot--green"></span>
    <span class="mockup__url">app.example.com/path</span>
  </div>
  <div class="mockup__body">
    <!-- hi-fi content using .ui-* components -->
  </div>
</figure>
```

---

## Update spec status (active → paused / completed / archived)

Edit the JSON in `<script id="spec-meta">`. Use canonical key order: `id`, `title`, `status`, `created`, `updated`, `priority`, `tags`, `mockup-fidelity` (logical, not alphabetical).

```html
<!-- Before -->
<script type="application/json" id="spec-meta">{"id":"team-invites","title":"Team Invites","status":"active","created":"2026-05-08","updated":"2026-05-12","priority":"high","tags":["feature"],"mockup-fidelity":"hi-fi"}</script>
```
```html
<!-- After -->
<script type="application/json" id="spec-meta">{"id":"team-invites","title":"Team Invites","status":"paused","created":"2026-05-08","updated":"2026-05-12","priority":"high","tags":["feature"],"mockup-fidelity":"hi-fi"}</script>
```

Also update the visible status pill in `region:header`:

```html
<!-- Before -->
<span class="pill pill--in-progress">Active</span>
```
```html
<!-- After -->
<span class="pill pill--pending">Paused</span>
```

---

## Update the `updated` date

Both in the JSON metadata AND in the visible header dl. Do them in the same edit transaction.

```html
<!-- In <script id="spec-meta">: change "updated":"OLD-DATE" to "updated":"NEW-DATE" -->

<!-- In the visible dl: -->
<div><dt>Updated</dt><dd>OLD-DATE</dd></div>
<!-- becomes -->
<div><dt>Updated</dt><dd>NEW-DATE</dd></div>
```

Then mirror the date into the matching row of `.specs/registry.md`.

---

## Append a tag

Two places — JSON and visible chips.

```html
<!-- JSON: change "tags":["feature","auth"] to "tags":["feature","auth","new-tag"] -->

<!-- Visible chips: -->
<dd><span class="chip">feature</span> <span class="chip">auth</span></dd>
<!-- becomes -->
<dd><span class="chip">feature</span> <span class="chip">auth</span> <span class="chip">new-tag</span></dd>
```

---

## Update the registry

`.specs/registry.md` is still markdown. After updating progress in `SPEC.html` (which just means a `data-status` swap), update the registry table row. The runtime derives progress in the HTML, but the registry is parsed by other tools so it needs its `X/Y` and `Updated` columns kept in sync.

```markdown
<!-- Before -->
| team-invites | Team Invites | active | high | 4/12 | 2026-05-11 |

<!-- After -->
| team-invites | Team Invites | active | high | 5/12 | 2026-05-12 |
```

---

## Validation after every edit

```bash
python3 -c "
import re, sys, json
p = sys.argv[1]; h = open(p).read()
m = re.search(r'<script[^>]*id=\"spec-meta\"[^>]*>(.+?)</script>', h, re.S); assert m; json.loads(m.group(1))
o = re.findall(r'<!--\s*region:(\w+)\s*-->', h); c = re.findall(r'<!--\s*endregion:(\w+)\s*-->', h); assert sorted(o)==sorted(c)
print('OK')
" .specs/<id>/SPEC.html
```

If it prints anything other than `OK`, fix the spec before continuing.
