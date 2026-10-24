# Hi-fi Mockup Library

Ready-to-use hi-fi UI patterns built from the `.ui-*` components in `spec-styles.css`. Copy a pattern into a `<figure class="mockup mockup--hifi">` and customize the content.

Hi-fi mockups use a **constrained palette** — only the design tokens defined in `spec-styles.css`. No arbitrary colors, no Tailwind utility classes. This is intentional: it kills the color/typography bikeshedding vector while still producing real-looking UI.

Use hi-fi mockups when the design is **locked** or when the spec is communicating a specific UX decision. Otherwise prefer wireframes (`wireframe-library.md`) — they're cheaper to author and don't overpromise.

## Components reference

See `spec-format.md` for the full list. Highlights:

- `.ui-app-shell` — sidebar + topbar + main layout
- `.ui-card` — content card with `.ui-card__title` / `.ui-card__sub`
- `.ui-button` — variants: `--primary`, `--danger`, `--ghost`, `--sm`, `--lg`
- `.ui-input` / `.ui-select` / `.ui-label` / `.ui-field` (with `.ui-field__error`)
- `.ui-table` — table with sticky header
- `.ui-modal` — modal card (wrap in `.ui-modal-backdrop` for backdrop look)
- `.ui-toast` — corner notification (variants `--success`, `--danger`)
- `.ui-alert` — inline alert (variants `--info`, `--warning`, `--danger`, `--success`)
- `.ui-empty-state` — empty state card with icon + heading + CTA
- `.ui-stepper` — horizontal progress steps (with `.ui-stepper__step--current` / `--done`)
- `.ui-tag` — pill/badge (variants `--success`, `--warning`, `--danger`, `--info`)
- `.ui-avatar` / `.ui-avatar--lg` / `.ui-avatar-group` — initials avatar
- `.ui-tabs` / `.ui-tab` / `.ui-tab--active` — tab strip
- `.ui-nav-item` / `.ui-nav-item--active` — sidebar/menu row
- `.ui-kbd` — keyboard shortcut chip

## Patterns

### Login / signup form

```html
<div style="min-height: 480px; display: grid; place-items: center; background: var(--subtle-bg); padding: var(--space-8);">
  <div class="ui-card" style="width: 360px;">
    <h3 class="ui-card__title">Sign in</h3>
    <div class="ui-field">
      <label class="ui-label">Email</label>
      <input class="ui-input" placeholder="you@company.com">
    </div>
    <div class="ui-field">
      <label class="ui-label">Password</label>
      <input class="ui-input" type="password" placeholder="••••••••">
    </div>
    <button class="ui-button ui-button--primary" style="width: 100%;">Continue</button>
    <p class="text-center mt-3"><a class="muted">Sign in with Google instead</a></p>
  </div>
</div>
```

### Dashboard with sidebar

```html
<div class="ui-app-shell">
  <div class="ui-topbar">
    <span class="ui-topbar__brand">Acme</span>
    <div class="flex items-center gap-2">
      <span class="ui-tag">Workspace 1</span>
      <span class="ui-avatar">GV</span>
    </div>
  </div>
  <aside class="ui-sidebar">
    <nav class="ui-nav">
      <a class="ui-nav-item">Overview</a>
      <a class="ui-nav-item ui-nav-item--active">Team</a>
      <a class="ui-nav-item">Billing</a>
      <a class="ui-nav-item">Settings</a>
    </nav>
  </aside>
  <main class="ui-main">
    <h2 class="mt-1 mb-3">Team</h2>
    <div class="ui-card">
      <h3 class="ui-card__title">Pending invites</h3>
      <p class="ui-card__sub">3 outstanding</p>
    </div>
  </main>
</div>
```

### Data table

```html
<div class="ui-card">
  <div class="flex items-center justify-between mb-3">
    <h3 class="ui-card__title">Pending invites</h3>
    <button class="ui-button ui-button--primary ui-button--sm">+ Invite teammate</button>
  </div>
  <table class="ui-table">
    <thead>
      <tr><th>Email</th><th>Role</th><th>Invited by</th><th>Expires</th><th></th></tr>
    </thead>
    <tbody>
      <tr>
        <td>daniela@acme.io</td>
        <td><span class="ui-tag ui-tag--info">admin</span></td>
        <td><span class="ui-avatar">GV</span> Gabriel</td>
        <td class="text-muted">in 6 days</td>
        <td><button class="ui-button ui-button--ghost ui-button--sm">Revoke</button></td>
      </tr>
      <tr>
        <td>marek@acme.io</td>
        <td><span class="ui-tag">member</span></td>
        <td><span class="ui-avatar">GV</span> Gabriel</td>
        <td class="text-muted">in 3 days</td>
        <td><button class="ui-button ui-button--ghost ui-button--sm">Revoke</button></td>
      </tr>
    </tbody>
  </table>
</div>
```

### Empty state

```html
<div class="ui-card">
  <div class="ui-empty-state">
    <div class="ui-empty-state__icon">✉</div>
    <h3 class="ui-empty-state__title">No pending invites</h3>
    <p class="ui-empty-state__text">You haven't invited anyone yet. Send your first invite to start building a team.</p>
    <button class="ui-button ui-button--primary">Invite first teammate</button>
  </div>
</div>
```

### Modal dialog

```html
<div class="ui-modal-backdrop">
  <div class="ui-modal">
    <div class="ui-modal__header">
      <h3 class="ui-modal__title">Revoke invite?</h3>
    </div>
    <div class="ui-modal__body">
      <p>The invitee will no longer be able to use this link. You can send a new invite at any time.</p>
    </div>
    <div class="ui-modal__footer">
      <button class="ui-button">Cancel</button>
      <button class="ui-button ui-button--danger">Revoke</button>
    </div>
  </div>
</div>
```

### Toast notification

```html
<div class="flex justify-between items-center" style="padding: var(--space-4); background: var(--subtle-bg); border-radius: var(--radius);">
  <div class="ui-toast ui-toast--success">✓ Invite sent to daniela@acme.io</div>
</div>
```

### Form with validation errors

```html
<div class="ui-card" style="max-width: 480px;">
  <h3 class="ui-card__title">Invite teammate</h3>
  <div class="ui-field">
    <label class="ui-label">Email</label>
    <input class="ui-input ui-input--error" value="not-an-email">
    <div class="ui-field__error">Enter a valid email address.</div>
  </div>
  <div class="ui-field">
    <label class="ui-label">Role</label>
    <select class="ui-select">
      <option>Member</option>
      <option>Admin</option>
    </select>
    <div class="ui-help">Admins can invite, remove, and change roles of other members.</div>
  </div>
  <button class="ui-button ui-button--primary">Send invite</button>
</div>
```

### Multi-step wizard

```html
<div class="ui-card">
  <div class="ui-stepper mb-3">
    <div class="ui-stepper__step ui-stepper__step--done"><span class="ui-stepper__bullet">1</span> Account</div>
    <div class="ui-stepper__sep"></div>
    <div class="ui-stepper__step ui-stepper__step--current"><span class="ui-stepper__bullet">2</span> Team</div>
    <div class="ui-stepper__sep"></div>
    <div class="ui-stepper__step"><span class="ui-stepper__bullet">3</span> Billing</div>
  </div>
  <h3 class="ui-card__title">Invite your team</h3>
  <div class="ui-field"><label class="ui-label">Email addresses (comma-separated)</label><input class="ui-input"></div>
  <div class="flex justify-between mt-3">
    <button class="ui-button">Back</button>
    <button class="ui-button ui-button--primary">Continue</button>
  </div>
</div>
```

### Alert + tabs

```html
<div class="ui-card">
  <div class="ui-tabs">
    <a class="ui-tab ui-tab--active">Active</a>
    <a class="ui-tab">Expired</a>
    <a class="ui-tab">All</a>
  </div>
  <div class="ui-alert ui-alert--warning mb-3">
    <span>⚠</span>
    <div>Some invites are about to expire. Resend them before they expire to avoid disruption.</div>
  </div>
  <p class="muted">3 active invites</p>
</div>
```

### Settings panel

```html
<div class="ui-card">
  <h3 class="ui-card__title">Workspace settings</h3>
  <div class="ui-field">
    <label class="ui-label">Workspace name</label>
    <input class="ui-input" value="Acme">
  </div>
  <div class="ui-field">
    <label class="ui-label">Default role for new invites</label>
    <select class="ui-select">
      <option>Member</option>
      <option>Admin</option>
    </select>
  </div>
  <hr style="border: 0; border-top: 1px solid var(--border-subtle); margin: var(--space-4) 0;">
  <h3 class="ui-card__title">Danger zone</h3>
  <p class="ui-card__sub">Deleting a workspace cannot be undone.</p>
  <button class="ui-button ui-button--danger">Delete workspace</button>
</div>
```

### Card grid

```html
<div style="display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-3);">
  <div class="ui-card">
    <h3 class="ui-card__title">Free</h3>
    <p class="ui-card__sub">Up to 3 teammates</p>
    <button class="ui-button" style="width: 100%;">Current plan</button>
  </div>
  <div class="ui-card" style="border-color: var(--accent);">
    <span class="ui-tag ui-tag--info">Popular</span>
    <h3 class="ui-card__title mt-2">Team</h3>
    <p class="ui-card__sub">Up to 25 teammates</p>
    <button class="ui-button ui-button--primary" style="width: 100%;">Upgrade</button>
  </div>
  <div class="ui-card">
    <h3 class="ui-card__title">Enterprise</h3>
    <p class="ui-card__sub">Unlimited teammates</p>
    <button class="ui-button" style="width: 100%;">Contact sales</button>
  </div>
</div>
```

## When to use which

- **Login form** — auth flows, signup pages
- **Dashboard** — overview / home view
- **Data table** — list of entities (users, invites, billing items)
- **Empty state** — zero-data view; design upfront because users will see it on day one
- **Modal** — confirmations, destructive actions, focused inline tasks
- **Toast** — transient success/failure feedback
- **Form with errors** — illustrating validation states
- **Wizard** — multi-step setup (onboarding, billing)
- **Alert + tabs** — informational content with filtered views
- **Settings panel** — grouped configuration with optional danger zone
- **Card grid** — pricing tiers, plan comparison, feature discovery

Mix hi-fi mockups with wireframes in the same spec if the design fidelity varies — e.g., wireframe the early/exploratory screens and hi-fi the critical user-facing ones.
